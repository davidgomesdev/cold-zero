//! Template project for Flipper Zero.
//! This app prints "Hello, Rust!" to the console then exits.

#![no_main]
#![no_std]

mod ir_timings;

// Required for panic handler
extern crate flipperzero_rt;

use core::ffi::CStr;

use flipperzero::notification::NotificationApp;
use flipperzero::notification;
use flipperzero_rt::{entry, manifest};
use flipperzero_sys::infrared_send_raw_ext;

manifest!(
    name = "ColdZero",
    app_version = 1,
    has_icon = true,
    // See https://github.com/flipperzero-rs/flipperzero/blob/v0.11.0/docs/icons.md for icon format
    icon = "rustacean-10x10.icon",
);

entry!(main);

fn run() {
    press_button(&ir_timings::POWER_BTN);

    let mut app = NotificationApp::open();

    app.notify(&notification::vibro::SINGLE_VIBRO);
}

fn press_button(timings: &[u32]) {
    unsafe {
        infrared_send_raw_ext(
            timings.as_ptr(),
            timings.len() as u32,
            true,
            38000,
            0.330000,
        );
    }
}

fn main(_args: Option<&CStr>) -> i32 {
    run();

    0
}
