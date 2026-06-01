#![no_main]
#![no_std]

// Required for panic handler
extern crate alloc;
extern crate flipperzero_rt;

mod allocator;
mod fan;
mod ir;
mod notification;
mod state;

use crate::fan::{FanLight, FanMode, FanState};
use crate::notification::{DAYTIME_CHANGE, MANUAL_POWER_OFF, MANUAL_POWER_ON};
use crate::state::{ActiveDevice, HeaterMode, HeaterState, RunState};
use alloc::alloc::{alloc, dealloc};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use core::alloc::Layout;
use core::ffi::{CStr, c_char, c_void};
use flipperzero::debug;
use flipperzero::furi::hal::rtc::datetime;
use flipperzero::notification::NotificationApp;
use flipperzero_rt::{entry, manifest};
use flipperzero_sys::{AlignBottom, AlignCenter, AlignRight, AlignTop, Canvas, FuriMessageQueue, FuriMutexTypeNormal, FuriStatusOk, FuriWaitForever, Gui, GuiLayerFullscreen, InputEvent, InputKeyBack, InputKeyDown, InputKeyLeft, InputKeyOk, InputKeyRight, InputTypeLong, InputTypeShort, ViewPort, ViewPortOrientationHorizontal, canvas_draw_str, canvas_draw_str_aligned, free, furi_message_queue_alloc, furi_message_queue_free, furi_message_queue_get, furi_message_queue_put, furi_mutex_acquire, furi_mutex_alloc, furi_mutex_free, furi_mutex_release, furi_record_close, furi_record_open, gui_add_view_port, gui_remove_view_port, view_port_alloc, view_port_draw_callback_set, view_port_enabled_set, view_port_free, view_port_input_callback_set, view_port_set_orientation, view_port_update, AlignLeft, furi_hal_power_shutdown, halt};
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
const START_HOUR_WEEKDAYS: u8 = 8;
const START_HOUR_WEEKENDS: u8 = 9;
const END_OF_START_HOUR: u8 = 13;

fn run() {
    unsafe {
        let queue = furi_message_queue_alloc(8, size_of::<InputEvent>() as u32);
        let view_port = view_port_alloc();
        let mut notification_app = NotificationApp::open();

        let app_state = Box::into_raw(Box::new(AppState {
            heater_state: HeaterState::default(),
            fan_state: FanState::default(),
            active_device: ActiveDevice::Heater,
            run_state: RunState::WaitingForDaytime,
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

        view_port_draw_callback_set(view_port, Some(on_draw), app_state.cast());
        view_port_input_callback_set(view_port, Some(on_input), queue.cast());
        view_port_set_orientation(view_port, ViewPortOrientationHorizontal);

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

#[allow(non_upper_case_globals)]
fn handle_key_presses(
    notification_app: &mut NotificationApp,
    view_port: *mut ViewPort,
    input_event: *mut InputEvent,
    app_state: &mut AppState,
) -> bool {
    unsafe {
        let input_event = *input_event;

        match input_event.key {
            InputKeyBack => {
                return false;
            }
            InputKeyOk => handle_ok_press(notification_app, app_state, input_event),
            InputKeyLeft | InputKeyRight => cycle_device(app_state),
            key => {
                debug!("Received input that is not handled ({})", key.0);
            }
        }

        view_port_update(view_port);
    }
    true
}

fn handle_ok_press(
    notification_app: &mut NotificationApp,
    app_state: &mut AppState,
    input_event: InputEvent,
) {
    match app_state.active_device {
        ActiveDevice::Heater => handle_heater_ok_press(notification_app, app_state, input_event),
        ActiveDevice::Fan => handle_fan_ok_press(app_state, input_event),
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
            app_state.run_state = RunState::Changing;
            app_state.heater_state.power_on();
            notification_app.notify(&MANUAL_POWER_ON);
        }
        InputTypeLong => {
            app_state.run_state = RunState::Changing;
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

#[allow(non_upper_case_globals)]
fn handle_fan_ok_press(app_state: &mut AppState, input_event: InputEvent) {
    if input_event.type_ != InputTypeShort && input_event.type_ != InputTypeLong {
        return;
    }
    if app_state.fan_state.is_on {
        app_state.fan_state.power_off();
    } else {
        app_state.fan_state.power_on();
    }
}

fn cycle_device(app_state: &mut AppState) {
    app_state.active_device = match app_state.active_device {
        ActiveDevice::Heater => ActiveDevice::Fan,
        ActiveDevice::Fan => ActiveDevice::Heater,
    };
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
        match app_state.active_device {
            ActiveDevice::Heater => draw_heater(canvas, app_state),
            ActiveDevice::Fan => draw_fan(canvas, app_state),
        }
    }
}

unsafe fn draw_heater(canvas: *mut Canvas, app_state: &AppState) {
    draw_header(canvas, app_state);

    let status = match app_state.run_state {
        RunState::WaitingForDaytime => c"Waiting for daytime...".as_ptr(),
        RunState::Changing => c"Changing heater state...".as_ptr(),
        RunState::SetDaytimeHeat => c"Heater set for daytime!".as_ptr(),
    };
    canvas_draw_str(canvas, 0, 20, status);

    let mode_str = match app_state.heater_state.mode {
        HeaterMode::HeatLow => "HeatLow",
        HeaterMode::HeatHigh => "HeatHigh",
        HeaterMode::Eco => "Eco",
    };
    let heater_str = format!(
        "Power: {} {}C {}",
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
}

unsafe fn draw_time(canvas: *mut Canvas) {
    let time_str = get_time_label();
    canvas_draw_str_aligned(canvas, 0, SCREEN_HEIGHT, AlignLeft, AlignBottom, time_str.as_ptr());
}

unsafe fn draw_header(canvas: *mut Canvas, app_state: &AppState) {
    let active_device_label = match app_state.active_device {
        ActiveDevice::Heater => c"< Heater >".as_ptr(),
        ActiveDevice::Fan => c"< Fan >".as_ptr(),
    };
    canvas_draw_str_aligned(
        canvas,
        SCREEN_WIDTH / 2,
        0,
        AlignCenter,
        AlignTop,
        active_device_label,
    );
}

unsafe fn draw_fan(canvas: *mut Canvas, app_state: &AppState) {
    draw_header(canvas, app_state);

    let on_str = if app_state.fan_state.is_on {
        c"Fan: ON".as_ptr()
    } else {
        c"Fan: OFF".as_ptr()
    };
    canvas_draw_str(canvas, 0, 20, on_str);

    if app_state.fan_state.is_on {
        let light_str = match app_state.fan_state.light {
            FanLight::Full => "Full",
            FanLight::Partial => "Part",
            FanLight::Off => "Off",
        };
        let mode_str = match app_state.fan_state.fan_mode {
            FanMode::F1 => "F1",
            FanMode::F2 => "F2",
            FanMode::F3 => "F3",
            FanMode::Sleep => "Slp",
            FanMode::Nature => "Nat",
        };
        let fan_detail = format!(
            "Light:{light_str} Timer:{}h {mode_str}\0",
            app_state.fan_state.timer,
        );
        canvas_draw_str(canvas, 0, 30, fan_detail.as_ptr());
    } else {
        canvas_draw_str(canvas, 0, 30, c"".as_ptr());
    }

    draw_time(canvas);

    let hints = if app_state.fan_state.is_on {
        c"OK:off".as_ptr()
    } else {
        c"OK:on".as_ptr()
    };
    draw_hints(canvas, hints);
}

unsafe fn draw_hints(canvas: *mut Canvas, hints: *const c_char) {
    canvas_draw_str_aligned(
        canvas,
        SCREEN_WIDTH,
        SCREEN_HEIGHT,
        AlignRight,
        AlignBottom,
        hints,
    );
}

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
