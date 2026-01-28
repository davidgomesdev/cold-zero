//! Template project for Flipper Zero.
//! This app prints "Hello, Rust!" to the console then exits.

#![no_main]
#![no_std]

// Required for panic handler
extern crate alloc;
extern crate flipperzero_rt;

mod allocator;
mod ir;
mod state;

use crate::state::{HeaterMode, HeaterState};
use alloc::alloc::alloc;
use alloc::boxed::Box;
use alloc::format;
use core::alloc::Layout;
use core::ffi::{CStr, c_char, c_void};
use flipperzero::debug;
use flipperzero::furi::hal::rtc::datetime;
use flipperzero_rt::{entry, manifest};
use flipperzero_sys::{
    Canvas, FuriMessageQueue, FuriMutexTypeNormal, FuriStatusOk, FuriWaitForever, Gui,
    GuiLayerFullscreen, InputEvent, InputKeyBack, InputKeyOk, InputTypeLong, InputTypeShort,
    ViewPort, ViewPortOrientationHorizontal, canvas_draw_str, free, furi_message_queue_alloc,
    furi_message_queue_free, furi_message_queue_get, furi_message_queue_put, furi_mutex_acquire,
    furi_mutex_alloc, furi_mutex_free, furi_mutex_release, furi_record_close, furi_record_open,
    gui_add_view_port, gui_remove_view_port, view_port_alloc, view_port_draw_callback_set,
    view_port_enabled_set, view_port_free, view_port_input_callback_set, view_port_set_orientation,
    view_port_update,
};
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
const START_HOUR_WEEKDAYS: u8 = 8;
const START_HOUR_WEEKENDS: u8 = 9;
const END_OF_START_HOUR: u8 = 13;

fn run() {
    unsafe {
        let queue = furi_message_queue_alloc(8, size_of::<InputEvent>() as u32);
        let view_port = view_port_alloc();

        let app_state = Box::into_raw(Box::new(AppState {
            heater_state: HeaterState::default(),
            last_called_day: 0,
            mutex: furi_mutex_alloc(FuriMutexTypeNormal),
        }));

        view_port_draw_callback_set(view_port, Some(on_draw), app_state.cast());
        view_port_input_callback_set(view_port, Some(on_input), queue.cast());
        view_port_set_orientation(view_port, ViewPortOrientationHorizontal);

        let gui: *mut Gui = furi_record_open(RECORD_GUI).cast();

        gui_add_view_port(gui, view_port, GuiLayerFullscreen);

        let input_event: *mut InputEvent = alloc(Layout::new::<InputEvent>()).cast();

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

            if time.hour < END_OF_START_HOUR
                && time.hour >= start_hour
                && app_state.last_called_day < time.day
            {
                start_of_day_power_heater(app_state);

                view_port_update(view_port);
                furi_mutex_release(app_state.mutex);

                continue;
            }

            if furi_message_queue_get(queue, input_event.cast(), 100) == FuriStatusOk {
                running = handle_key_presses(view_port, input_event, app_state);
            }

            furi_mutex_release(app_state.mutex);
        }

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
            InputKeyOk => handle_ok_press(app_state, input_event),
            key => {
                debug!("Received input that is not handled ({})", key.0);
            }
        }

        view_port_update(view_port);
    }
    true
}

#[allow(non_upper_case_globals)]
fn handle_ok_press(app_state: &mut AppState, input_event: InputEvent) {
    if (input_event.type_ == InputTypeLong || input_event.type_ == InputTypeShort)
        && app_state.heater_state.is_on
    {
        app_state.heater_state._power_off();
        return;
    }

    match input_event.type_ {
        InputTypeShort => {
            app_state.heater_state.power_on();
        }
        InputTypeLong => {
            start_of_day_power_heater(app_state);
        }
        _ => {
            debug!("OK button press type not handled ({})", input_event.type_.0);
        }
    }
}

fn start_of_day_power_heater(app_state: &mut AppState) {
    let heater_state = &mut app_state.heater_state;

    heater_state.power_on();
    heater_state.change_mode(HeaterMode::HeatHigh);
    heater_state.set_temp(35);

    app_state.last_called_day = datetime().day;
}

unsafe extern "C" fn on_draw(canvas: *mut Canvas, app_state: *mut c_void) {
    unsafe {
        let app_state: &AppState = &mut *(app_state as *mut AppState);

        canvas_draw_str(canvas, 0, 10, c"-- Cold Zero --".as_ptr());

        let text = if app_state.last_called_day == datetime().day {
            c"Already ran today!"
        } else {
            c"Waiting to run..."
        };

        canvas_draw_str(canvas, 0, 20, text.as_ptr());

        canvas_draw_str(
            canvas,
            0,
            60,
            format!(
                "Current Time: {}:{}:{}",
                datetime().hour,
                datetime().minute,
                datetime().second
            )
            .as_ptr(),
        );

        canvas_draw_str(
            canvas,
            0,
            30,
            format!(
                "Heater state: {} {} {:?}",
                if app_state.heater_state.is_on {
                    "ON"
                } else {
                    "OFF"
                },
                app_state.heater_state.temperature,
                app_state.heater_state.mode
            )
            .as_ptr(),
        );
    }
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
