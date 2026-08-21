#![no_main]
#![no_std]

// Required for panic handler
extern crate alloc;
extern crate flipperzero_rt;

mod ac;
mod allocator;
mod bulbs;
mod daikin;
mod fan;
mod icons;
mod ir;
mod notification;
mod state;

use crate::ac::{FIELDS, Field, Picker};
use crate::bulbs::BulbsState;
use crate::fan::{FanLight, FanMode, FanSpeed, FanState};
use crate::notification::{DAYTIME_CHANGE, MANUAL_POWER_OFF, MANUAL_POWER_ON};
use crate::state::{ActiveDevice, DEVICES, HeaterMode, HeaterState, RunState};
use ac::AcState;
use alloc::alloc::{alloc, dealloc};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use core::alloc::Layout;
use core::ffi::{CStr, c_char, c_void};
use flipperzero::debug;
use flipperzero::furi::hal::rtc::datetime;
use flipperzero::notification::NotificationApp;
use flipperzero::notification::led::{BLINK_START_BLUE, BLINK_STOP};
use flipperzero_rt::{entry, manifest};
use flipperzero_sys::{AlignBottom, AlignCenter, AlignLeft, AlignRight, AlignTop, Canvas, ColorBlack, ColorWhite, FontPrimary, FontSecondary, FuriMessageQueue, FuriMutexTypeNormal, FuriStatusOk, FuriWaitForever, Gui, GuiLayerFullscreen, InputEvent, InputKeyBack, InputKeyDown, InputKeyLeft, InputKeyOk, InputKeyRight, InputKeyUp, InputTypeLong, InputTypeShort, ViewPort, ViewPortOrientationHorizontal, ViewPortOrientationVertical, canvas_draw_box, canvas_draw_disc, canvas_draw_rframe, canvas_draw_str, canvas_draw_str_aligned, canvas_draw_xbm, canvas_set_color, canvas_set_font, free, furi_message_queue_alloc, furi_message_queue_free, furi_message_queue_get, furi_message_queue_put, furi_mutex_acquire, furi_mutex_alloc, furi_mutex_free, furi_mutex_release, furi_record_close, furi_record_open, gui_add_view_port, gui_remove_view_port, view_port_alloc, view_port_draw_callback_set, view_port_enabled_set, view_port_free, view_port_input_callback_set, view_port_set_orientation, view_port_update};
use state::AppState;

manifest!(
    name = "ColdZero",
    app_version = 1,
    has_icon = true,
    // See https://github.com/flipperzero-rs/flipperzero/blob/v0.11.0/docs/icons.md for icon format
    icon = "rustacean-10x10.icon",
);

entry!(main);

const RECORD_GUI: *const c_char = c"gui".as_ptr();
const SCREEN_WIDTH: i32 = 127;
const SCREEN_HEIGHT: i32 = 63;
/// The A/C screen is the only one rendered sideways, so it gets the other
/// dimensions.
const AC_WIDTH: i32 = 63;
const AC_HEIGHT: i32 = 127;
/// One home tile, half the screen each way.
const TILE_WIDTH: i32 = 64;
const TILE_HEIGHT: i32 = 32;
/// Column the bulb ON/OFF values start at, so they line up under each other.
/// Wide enough to clear "Escritorio:" in either of the stock fonts.
const BULB_VALUE_X: i32 = 70;
/// Second column of the wiring row under the two bulbs.
const BULB_WIRING_X: i32 = 66;
/// Animation frames for the label shown while a button sequence goes out.
const CHANGING: [&CStr; 4] = [c"Changing", c"Changing.", c"Changing..", c"Changing..."];
const START_HOUR_WEEKDAYS: u8 = 8;
const START_HOUR_WEEKENDS: u8 = 9;
const END_OF_START_HOUR: u8 = 13;

fn run() {
    unsafe {
        let queue = furi_message_queue_alloc(8, size_of::<InputEvent>() as u32);
        let view_port = view_port_alloc();
        let mut notification_app = NotificationApp::open();

        let app_state = Box::into_raw(Box::new(AppState {
            ac_state: AcState::load(),
            heater_state: HeaterState::default(),
            fan_state: FanState::default(),
            bulbs_state: BulbsState::default(),
            // The A/C is the one that gets used year round, so the app skips
            // the home screen and opens straight into it.
            active_device: ActiveDevice::Ac,
            in_device: true,
            run_state: RunState::WaitingForDaytime,
            sending: false,
            last_daytime_run_day: 0,
            mutex: furi_mutex_alloc(FuriMutexTypeNormal),
        }));

        // If the app is opened when the trigger window has already started (or passed),
        // mark today as handled so the heater doesn't fire immediately on launch.
        {
            let time = datetime();
            let start_hour = if time.weekday > 5 {
                START_HOUR_WEEKENDS
            } else {
                START_HOUR_WEEKDAYS
            };
            if time.hour >= start_hour {
                (*app_state).last_daytime_run_day = time.day;
            }
        }

        ir::set_view_port(view_port);
        view_port_draw_callback_set(view_port, Some(on_draw), app_state.cast());
        view_port_input_callback_set(view_port, Some(on_input), queue.cast());
        apply_orientation(view_port, &*app_state);

        bulbs::init();

        let gui: *mut Gui = furi_record_open(RECORD_GUI).cast();

        gui_add_view_port(gui, view_port, GuiLayerFullscreen);

        let input_event_layout = Layout::new::<InputEvent>();
        let input_event: *mut InputEvent = alloc(input_event_layout).cast();

        let mut running = true;

        while running {
            furi_mutex_acquire((*app_state).mutex, FuriWaitForever.0);

            let time = datetime();
            let start_hour = if time.weekday > 5 {
                START_HOUR_WEEKENDS
            } else {
                START_HOUR_WEEKDAYS
            };
            let app_state = app_state.as_mut().expect("App state is null!");

            if app_state.last_daytime_run_day < time.day
                && app_state.in_device
                && app_state.active_device == ActiveDevice::Heater
            {
                if time.hour < END_OF_START_HOUR && time.hour >= start_hour {
                    start_of_day_power_heater(&mut notification_app, app_state);

                    app_state.run_state = RunState::SetDaytimeHeat;
                    view_port_update(view_port);
                    furi_mutex_release(app_state.mutex);

                    continue;
                }

                if app_state.run_state != RunState::WaitingForDaytime {
                    app_state.run_state = RunState::WaitingForDaytime;
                    view_port_update(view_port);
                }
            }

            if furi_message_queue_get(queue, input_event.cast(), 100) == FuriStatusOk {
                running =
                    handle_key_presses(&mut notification_app, view_port, input_event, app_state);
            }

            furi_mutex_release(app_state.mutex);
        }

        bulbs::deinit();

        dealloc(input_event as *mut u8, input_event_layout);
        view_port_enabled_set(view_port, false);
        furi_message_queue_free(queue);
        gui_remove_view_port(gui, view_port);
        view_port_free(view_port);
        furi_record_close(RECORD_GUI);
        furi_mutex_free((*app_state).mutex);
        free(app_state.cast());
    }
}

/// Only the A/C list is rendered sideways. The firmware rotates the input keys
/// to match, so every handler keeps working in logical directions.
unsafe fn apply_orientation(view_port: *mut ViewPort, app_state: &AppState) {
    let orientation = if app_state.in_device && app_state.active_device == ActiveDevice::Ac {
        ViewPortOrientationVertical
    } else {
        ViewPortOrientationHorizontal
    };
    unsafe { view_port_set_orientation(view_port, orientation) };
}

#[allow(non_upper_case_globals)]
fn handle_key_presses(
    notification_app: &mut NotificationApp,
    view_port: *mut ViewPort,
    input_event: *mut InputEvent,
    app_state: &mut AppState,
) -> bool {
    unsafe {
        let input_event = *input_event;

        // Back means the same thing everywhere: leave whatever you're in.
        if input_event.key == InputKeyBack {
            match input_event.type_ {
                // Holding quits from anywhere, including mid-screen.
                InputTypeLong => return false,
                // A tap backs out one level. On the home screen there is no
                // level left, so it leaves the app.
                InputTypeShort => {
                    if app_state.in_device
                        && app_state.active_device == ActiveDevice::Ac
                        && app_state.ac_state.close_menu()
                    {
                        // The mode picker swallowed it.
                    } else if !app_state.in_device {
                        return false;
                    } else {
                        app_state.in_device = false;
                        apply_orientation(view_port, app_state);
                    }
                }
                _ => {}
            }
            view_port_update(view_port);
            return true;
        }

        if !app_state.in_device {
            handle_home(view_port, app_state, input_event);
            view_port_update(view_port);
            return true;
        }

        // Anything that only moves a cursor must not raise the "Changing..."
        // flag or start the blink. The A/C answers for itself, since whether a
        // key transmits depends on the row and the picker.
        let sends = match app_state.active_device {
            ActiveDevice::Ac => app_state.ac_state.sends(input_event.key, input_event.type_),
            device => key_sends(device, input_event.key),
        };

        if !sends {
            if app_state.active_device == ActiveDevice::Ac {
                app_state.ac_state.navigate(input_event.key, input_event.type_);
            } else {
                debug!("Received input that is not handled ({})", input_event.key.0);
            }
            view_port_update(view_port);
            return true;
        }

        // The sends below block this thread for the whole sequence;
        // paint "Changing..." first so the screen isn't left frozen on
        // stale state while they go out.
        app_state.sending = true;
        // The notification service blinks on its own thread, so it
        // keeps going for the whole sequence while this one blocks.
        notification_app.notify(&BLINK_START_BLUE);
        view_port_update(view_port);

        if input_event.key == InputKeyOk {
            handle_ok_press(notification_app, app_state, input_event);
        } else {
            match app_state.active_device {
                ActiveDevice::Ac => handle_ac_control(app_state, input_event),
                ActiveDevice::Fan => handle_fan_control(app_state, input_event),
                ActiveDevice::Bulbs => handle_bulbs_control(app_state, input_event),
                ActiveDevice::Heater => {}
            }
        }

        app_state.sending = false;
        notification_app.notify(&BLINK_STOP);

        view_port_update(view_port);
    }
    true
}

/// Which keys put something on the air, for the screens whose answer depends
/// only on the key. The A/C has `AcState::sends` instead.
#[allow(non_upper_case_globals)]
fn key_sends(device: ActiveDevice, key: flipperzero_sys::InputKey) -> bool {
    match (device, key) {
        (_, InputKeyOk) => true,
        (ActiveDevice::Fan | ActiveDevice::Bulbs, InputKeyUp | InputKeyDown) => true,
        _ => false,
    }
}

/// Arrows walk the 2x2 tile grid, OK opens the tile.
#[allow(non_upper_case_globals)]
fn handle_home(view_port: *mut ViewPort, app_state: &mut AppState, input_event: InputEvent) {
    if input_event.type_ != InputTypeShort {
        return;
    }

    match input_event.key {
        InputKeyLeft | InputKeyRight => {
            app_state.active_device = app_state.active_device.step(false)
        }
        InputKeyUp | InputKeyDown => app_state.active_device = app_state.active_device.step(true),
        InputKeyOk => {
            app_state.in_device = true;
            unsafe { apply_orientation(view_port, app_state) };
        }
        _ => {}
    }
}

fn handle_ok_press(
    notification_app: &mut NotificationApp,
    app_state: &mut AppState,
    input_event: InputEvent,
) {
    match app_state.active_device {
        ActiveDevice::Ac => handle_ac_ok_press(app_state, input_event),
        ActiveDevice::Heater => handle_heater_ok_press(notification_app, app_state, input_event),
        ActiveDevice::Fan => handle_fan_ok_press(app_state, input_event),
        ActiveDevice::Bulbs => handle_bulbs_ok_press(app_state, input_event),
    }
    app_state.run_state = RunState::WaitingForDaytime;
}

#[allow(non_upper_case_globals)]
fn handle_heater_ok_press(
    notification_app: &mut NotificationApp,
    app_state: &mut AppState,
    input_event: InputEvent,
) {
    if (input_event.type_ == InputTypeLong || input_event.type_ == InputTypeShort)
        && app_state.heater_state.is_on
    {
        app_state.heater_state.power_off();
        notification_app.notify(&MANUAL_POWER_OFF);
        return;
    }

    match input_event.type_ {
        InputTypeShort => {
            app_state.heater_state.power_on();
            notification_app.notify(&MANUAL_POWER_ON);
        }
        InputTypeLong => {
            start_of_day_power_heater(notification_app, app_state);
        }
        _ => {
            debug!(
                "Received OK button press type not handled ({})",
                input_event.type_.0
            );
        }
    }
}

/// OK toggles power. Holding it resends the state unchanged, which is the only
/// way back in sync after someone has used the physical remote.
#[allow(non_upper_case_globals)]
fn handle_ac_ok_press(app_state: &mut AppState, input_event: InputEvent) {
    if app_state.ac_state.menu.is_some() {
        app_state.ac_state.commit_menu();
        return;
    }

    match input_event.type_ {
        InputTypeShort => app_state.ac_state.toggle_power(),
        InputTypeLong => app_state.ac_state.send(),
        _ => {}
    }
}

/// Left/Right change the selected row. Every change retransmits the whole
/// state, because that is all the Daikin protocol can say.
fn handle_ac_control(app_state: &mut AppState, input_event: InputEvent) {
    app_state
        .ac_state
        .adjust(input_event.key == InputKeyRight, input_event.type_);
}

#[allow(non_upper_case_globals)]
fn handle_fan_ok_press(app_state: &mut AppState, input_event: InputEvent) {
    if app_state.fan_state.is_on {
        if input_event.type_ == InputTypeShort || input_event.type_ == InputTypeLong {
            app_state.fan_state.power_off();
        }
        return;
    }

    match input_event.type_ {
        InputTypeShort => app_state.fan_state.power_on(),
        InputTypeLong => app_state.fan_state.power_on_full(),
        _ => {}
    }
}

/// Up toggles the escritorio, Down the quarto. Both fire on release, so holding
/// either one does nothing extra.
#[allow(non_upper_case_globals)]
fn handle_bulbs_control(app_state: &mut AppState, input_event: InputEvent) {
    if input_event.type_ != InputTypeShort {
        return;
    }

    match input_event.key {
        InputKeyUp => app_state.bulbs_state.toggle_escritorio(),
        InputKeyDown => app_state.bulbs_state.toggle_quarto(),
        _ => {}
    }
}

/// OK drives the pair: on unless both are already on, otherwise off.
fn handle_bulbs_ok_press(app_state: &mut AppState, input_event: InputEvent) {
    if input_event.type_ != InputTypeShort && input_event.type_ != InputTypeLong {
        return;
    }

    let all_on = app_state.bulbs_state.both_on();
    app_state.bulbs_state.set_both(!all_on);
}

/// Up/Down control the fan only, and only while it's running.
/// Up cycles speed and holding it steps the timer; Down toggles rotation and
/// holding it cycles mode.
#[allow(non_upper_case_globals)]
fn handle_fan_control(app_state: &mut AppState, input_event: InputEvent) {
    if !app_state.fan_state.is_on {
        return;
    }

    match (input_event.key, input_event.type_) {
        (InputKeyUp, InputTypeShort) => app_state.fan_state.next_speed(),
        (InputKeyUp, InputTypeLong) => app_state.fan_state.next_timer(),
        (InputKeyDown, InputTypeShort) => app_state.fan_state.rotate(),
        (InputKeyDown, InputTypeLong) => app_state.fan_state.next_mode(),
        _ => {}
    }
}

fn start_of_day_power_heater(notification_app: &mut NotificationApp, app_state: &mut AppState) {
    let heater_state = &mut app_state.heater_state;

    heater_state.power_on();
    heater_state.change_mode(HeaterMode::HeatHigh);
    heater_state.set_temp(35);

    app_state.last_daytime_run_day = datetime().day;
    notification_app.notify(&DAYTIME_CHANGE);
}

unsafe extern "C" fn on_draw(canvas: *mut Canvas, app_state: *mut c_void) {
    unsafe {
        let app_state: &AppState = &*(app_state as *const AppState);
        canvas_set_font(canvas, FontSecondary);

        if !app_state.in_device {
            draw_home(canvas, app_state);
            return;
        }

        match app_state.active_device {
            ActiveDevice::Ac => draw_ac(canvas, app_state),
            ActiveDevice::Heater => draw_heater(canvas, app_state),
            ActiveDevice::Fan => draw_fan(canvas, app_state),
            ActiveDevice::Bulbs => draw_bulbs(canvas, app_state),
        }

        if app_state.sending {
            // Bumped by ir_press_button, so the dots step once per frame sent
            let frame = ir::send_count() as usize % CHANGING.len();
            let y = if app_state.active_device == ActiveDevice::Ac {
                // Clear of the bottom edge: the baseline has to leave room for
                // the descender in "Changing".
                AC_HEIGHT - 4
            } else {
                50
            };
            canvas_draw_str(canvas, 0, y, CHANGING[frame].as_ptr());
        }
    }
}

/// Four tiles, one per device, selected one framed.
unsafe fn draw_home(canvas: *mut Canvas, app_state: &AppState) {
    for (index, device) in DEVICES.iter().enumerate() {
        let x = (index as i32 % 2) * TILE_WIDTH;
        let y = (index as i32 / 2) * TILE_HEIGHT;

        let (icon, label) = match device {
            ActiveDevice::Ac => (&icons::AC, c"A/C"),
            ActiveDevice::Heater => (&icons::HEATER, c"Heater"),
            ActiveDevice::Fan => (&icons::FAN, c"Fan"),
            ActiveDevice::Bulbs => (&icons::BULBS, c"Bulbs"),
        };

        unsafe {
            canvas_draw_xbm(
                canvas,
                x + (TILE_WIDTH - icons::SIZE as i32) / 2,
                y + 3,
                icons::SIZE,
                icons::SIZE,
                icon.as_ptr(),
            );
            canvas_draw_str_aligned(
                canvas,
                x + TILE_WIDTH / 2,
                y + TILE_HEIGHT - 3,
                AlignCenter,
                AlignBottom,
                label.as_ptr(),
            );

            if *device == app_state.active_device {
                canvas_draw_rframe(canvas, x + 1, y + 1, TILE_WIDTH as usize - 2, TILE_HEIGHT as usize - 2, 3);
            }
        }
    }
}

/// The A/C list, rendered sideways so all eleven settings fit at once.
/// One option row in a picker: 16px of icon plus its name.
const MENU_ROW_HEIGHT: i32 = 21;
const MENU_FIRST_ROW: i32 = 18;

/// A picker's options with an icon each, replacing the settings list while it
/// is open. Up/Down move, OK picks, Back closes.
unsafe fn draw_menu(canvas: *mut Canvas, ac: &AcState, picker: Picker, selected: usize) {
    unsafe {
        canvas_set_font(canvas, FontPrimary);
        canvas_draw_str_aligned(
            canvas,
            AC_WIDTH / 2,
            0,
            AlignCenter,
            AlignTop,
            picker.title().as_ptr(),
        );
        canvas_set_font(canvas, FontSecondary);

        for index in 0..picker.len() {
            let top = MENU_FIRST_ROW + index as i32 * MENU_ROW_HEIGHT;
            let (icon, label) = picker.option(index);

            canvas_draw_xbm(canvas, 5, top + 2, icons::SIZE, icons::SIZE, icon.as_ptr());
            canvas_draw_str_aligned(canvas, 27, top + 14, AlignLeft, AlignBottom, label.as_ptr());

            // The option in effect gets a dot, so the cursor isn't the only
            // thing on screen and you can see what you'd be changing away from.
            if index == picker.current(&ac.daikin) {
                canvas_draw_disc(canvas, AC_WIDTH - 6, top + 10, 2);
            }
            if index == selected {
                canvas_draw_rframe(canvas, 1, top, AC_WIDTH as usize - 1, 20, 3);
            }
        }
    }
}

unsafe fn draw_ac(canvas: *mut Canvas, app_state: &AppState) {
    let ac = &app_state.ac_state;

    if let Some((picker, selected)) = ac.menu {
        unsafe { draw_menu(canvas, ac, picker, selected) };
        return;
    }

    unsafe {
        canvas_set_font(canvas, FontPrimary);
        let title = if ac.daikin.power() {
            c"A/C ON".as_ptr()
        } else {
            c"A/C OFF".as_ptr()
        };
        canvas_draw_str_aligned(canvas, AC_WIDTH / 2, 0, AlignCenter, AlignTop, title);
        canvas_set_font(canvas, FontSecondary);

        for (index, field) in FIELDS.iter().enumerate() {
            let y = 20 + index as i32 * 9;

            if *field == ac.field {
                canvas_draw_box(canvas, 0, y - 7, AC_WIDTH as usize + 1, 9);
                canvas_set_color(canvas, ColorWhite);
            }

            canvas_draw_str_aligned(canvas, 1, y, AlignLeft, AlignBottom, field.label().as_ptr());

            let picker = field.picker();
            let temp;
            let value = match picker {
                // A picker row shows whichever option is in effect.
                Some(picker) => ac.picker_label(picker).as_ptr(),
                None => match (field, ac.flag(*field)) {
                    (_, Some(on)) => on_off(on),
                    (Field::Fan, _) => ac.fan_label().as_ptr(),
                    (Field::Temp, _) => {
                        temp = format!("{}C\0", ac.daikin.temp());
                        temp.as_ptr()
                    }
                    _ => c"".as_ptr(),
                },
            };
            // A picker row opens a submenu rather than changing in place, so
            // it says so.
            let value_x = if picker.is_some() {
                canvas_draw_str_aligned(canvas, AC_WIDTH, y, AlignRight, AlignBottom, c">".as_ptr());
                AC_WIDTH - 5
            } else {
                AC_WIDTH
            };
            canvas_draw_str_aligned(canvas, value_x, y, AlignRight, AlignBottom, value);

            canvas_set_color(canvas, ColorBlack);
        }
    }
}

unsafe fn draw_heater(canvas: *mut Canvas, app_state: &AppState) { unsafe {
    draw_title(canvas, c"Heater");

    let status = match app_state.run_state {
        RunState::WaitingForDaytime => c"Waiting for daytime...".as_ptr(),
        RunState::SetDaytimeHeat => c"Heater set for daytime!".as_ptr(),
    };
    canvas_draw_str(canvas, 0, 20, status);

    let mode_str = match app_state.heater_state.mode {
        HeaterMode::HeatLow => "HeatLow",
        HeaterMode::HeatHigh => "HeatHigh",
        HeaterMode::Eco => "Eco",
    };
    let heater_str = format!(
        "Power: {} {}C {}\0",
        if app_state.heater_state.is_on {
            "ON"
        } else {
            "OFF"
        },
        app_state.heater_state.temperature,
        mode_str,
    );
    canvas_draw_str(canvas, 0, 30, heater_str.as_ptr());

    draw_time(canvas);

    let hints = if app_state.heater_state.is_on {
        c"OK:off".as_ptr()
    } else {
        c"OK:on Hold:day".as_ptr()
    };
    draw_hints(canvas, hints);
}}

unsafe fn draw_time(canvas: *mut Canvas) { unsafe {
    let time_str = get_time_label();
    canvas_draw_str_aligned(canvas, 0, SCREEN_HEIGHT, AlignLeft, AlignBottom, time_str.as_ptr());
}}

unsafe fn draw_title(canvas: *mut Canvas, title: &CStr) { unsafe {
    canvas_draw_str_aligned(
        canvas,
        SCREEN_WIDTH / 2,
        0,
        AlignCenter,
        AlignTop,
        title.as_ptr(),
    );
}}

unsafe fn draw_fan(canvas: *mut Canvas, app_state: &AppState) { unsafe {
    draw_title(canvas, c"Fan");

    if app_state.fan_state.is_on {
        let on_str = format!(
            "Fan: ON Rot:{}\0",
            if app_state.fan_state.rotating {
                "on"
            } else {
                "off"
            },
        );
        canvas_draw_str(canvas, 0, 20, on_str.as_ptr());
    } else {
        canvas_draw_str(canvas, 0, 20, c"Fan: OFF".as_ptr());
    }

    if app_state.fan_state.is_on {
        let light_str = match app_state.fan_state.light {
            FanLight::Full => "Full",
            FanLight::Partial => "Part",
            FanLight::Off => "Off",
        };
        let mode_str = match app_state.fan_state.mode {
            FanMode::Normal => "Normal",
            FanMode::Sleep => "Sleep",
            FanMode::Nature => "Nature",
        };
        let speed_str = match app_state.fan_state.speed {
            FanSpeed::F1 => "F1",
            FanSpeed::F2 => "F2",
            FanSpeed::F3 => "F3",
            FanSpeed::SF => "SF",
        };
        let fan_mode = format!("Mode {mode_str} {speed_str}\0");
        canvas_draw_str(canvas, 0, 30, fan_mode.as_ptr());

        let fan_detail = format!("Light:{light_str} Timer:{}h\0", app_state.fan_state.timer);
        canvas_draw_str(canvas, 0, 40, fan_detail.as_ptr());

        // The hold bindings have nowhere else to go: the bottom line is
        // already the clock plus the short-press hints.
        if !app_state.sending {
            canvas_draw_str(canvas, 0, 50, c"hU:timer hD:mode".as_ptr());
        }
    } else {
        canvas_draw_str(canvas, 0, 30, c"".as_ptr());
    }

    draw_time(canvas);

    let hints = if app_state.fan_state.is_on {
        c"U:spd D:rot".as_ptr()
    } else {
        c"OK:on Hold:all".as_ptr()
    };
    draw_hints(canvas, hints);
}}

unsafe fn draw_bulbs(canvas: *mut Canvas, app_state: &AppState) { unsafe {
    draw_title(canvas, c"Bulbs");

    canvas_draw_str(canvas, 0, 20, c"Escrit\xf3rio:".as_ptr());
    canvas_draw_str(
        canvas,
        BULB_VALUE_X,
        20,
        on_off(app_state.bulbs_state.escritorio),
    );

    canvas_draw_str(canvas, 0, 30, c"Quarto:".as_ptr());
    canvas_draw_str(canvas, BULB_VALUE_X, 30, on_off(app_state.bulbs_state.quarto));

    // Static wiring reference, in the same order as the rows above. It gets its
    // own row because both pins per row overrun 127px in the wider stock font.
    canvas_draw_str(canvas, 0, 40, bulbs::ESCRITORIO_WIRING.as_ptr());
    canvas_draw_str(canvas, BULB_WIRING_X, 40, bulbs::QUARTO_WIRING.as_ptr());

    draw_time(canvas);

    let hints = if app_state.bulbs_state.both_on() {
        c"U:Esc D:Qua OK:off".as_ptr()
    } else {
        c"U:Esc D:Qua OK:on".as_ptr()
    };
    draw_hints(canvas, hints);
}}

fn on_off(on: bool) -> *const c_char {
    if on { c"ON".as_ptr() } else { c"OFF".as_ptr() }
}

unsafe fn draw_hints(canvas: *mut Canvas, hints: *const c_char) { unsafe {
    canvas_draw_str_aligned(
        canvas,
        SCREEN_WIDTH,
        SCREEN_HEIGHT,
        AlignRight,
        AlignBottom,
        hints,
    );
}}

fn get_time_label() -> String {
    let time = datetime();

    format!("{}:{}:{}\0", time.hour, time.minute, time.second,)
}

unsafe extern "C" fn on_input(input: *mut InputEvent, context: *mut c_void) {
    unsafe {
        let queue: *mut FuriMessageQueue = context.cast();
        furi_message_queue_put(queue, input.cast(), FuriWaitForever.0);
    }
}

fn main(_args: Option<&CStr>) -> i32 {
    run();

    0
}
