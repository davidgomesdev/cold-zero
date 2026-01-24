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
use core::sync::atomic::Ordering;
use flipperzero::furi::hal::rtc::datetime;
use flipperzero_rt::{entry, manifest};
use flipperzero_sys::{
    Canvas, FuriMessageQueue, FuriMutexTypeNormal, FuriStatusOk, FuriWaitForever, Gui,
    GuiLayerFullscreen, InputEvent, InputKeyBack, InputTypeRepeat, InputTypeShort,
    ViewPortOrientationHorizontal, canvas_draw_str, free, furi_message_queue_alloc,
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
const START_HOUR: u8 = 9;

fn run() {
    unsafe {
        let queue = furi_message_queue_alloc(8, size_of::<InputEvent>() as u32);
        let view_port = view_port_alloc();

        let app_state = Box::into_raw(Box::new(AppState {
            heater_state: HeaterState::default(),
            last_called_day: 0.into(),
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

            let last_called_day = (*app_state).last_called_day.load(Ordering::SeqCst);
            let time = datetime();

            if time.hour >= START_HOUR && last_called_day < time.day {
                let heater_state = &mut (*app_state).heater_state;

                start_of_day_power_heater(
                    app_state.as_ref().expect("App state is null!"),
                    heater_state,
                );

                view_port_update(view_port);
                furi_mutex_release((*app_state).mutex);

                continue;
            }

            if furi_message_queue_get(queue, input_event.cast(), 100) == FuriStatusOk {
                let input_event = *input_event;

                if (input_event.type_ == InputTypeShort || input_event.type_ == InputTypeRepeat)
                    && input_event.key == InputKeyBack {
                        running = false
                    }

                view_port_update(view_port);
            }

            furi_mutex_release((*app_state).mutex);
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

fn start_of_day_power_heater(app_state: &AppState, heater_state: &mut HeaterState) {
    heater_state.power_on();
    heater_state.change_mode(HeaterMode::HeatHigh);
    heater_state.set_temp(35);

    app_state.last_called_day.fetch_add(1, Ordering::SeqCst);
}

unsafe extern "C" fn on_draw(canvas: *mut Canvas, app_state: *mut c_void) {
    unsafe {
        let app_state: &AppState = &mut *(app_state as *mut AppState);

        canvas_draw_str(canvas, 0, 10, c"-- Cold Zero --".as_ptr());
        let last_called_day = app_state.last_called_day.load(Ordering::SeqCst);

        let text = if last_called_day == datetime().day {
            c"Already ran today!"
        } else {
            c"Waiting to run..."
        };

        canvas_draw_str(canvas, 0, 20, text.as_ptr());

        canvas_draw_str(
            canvas,
            0,
            50,
            format!(
                "Current Time: {}:{}:{}",
                datetime().hour,
                datetime().minute,
                datetime().second
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
