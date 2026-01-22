//! Template project for Flipper Zero.
//! This app prints "Hello, Rust!" to the console then exits.

#![no_main]
#![no_std]

// Required for panic handler
extern crate flipperzero_rt;
mod ir_timings;

use crate::ir_timings::{DUTY_CYCLE, FREQUENCY, POWER_BTN};
use core::ffi::CStr;
use core::time::Duration;
use flipperzero::furi::hal::rtc::datetime;
use flipperzero::furi::thread::sleep;
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
    let mut has_run = false;

    loop {
        sleep(Duration::from_mins(5));

        let current_time = datetime();

        if current_time.hour == 9 && !has_run {
            press_button(&POWER_BTN);
            has_run = true;
        }

        if current_time.hour != 22 && has_run {
            has_run = false;
        }
    }
}

fn press_button(timings: &[u32]) {
    unsafe {
        infrared_send_raw_ext(
            timings.as_ptr(),
            timings.len() as u32,
            true,
            FREQUENCY,
            DUTY_CYCLE,
        );
    }
}

fn main(_args: Option<&CStr>) -> i32 {
    run();

    0
}
