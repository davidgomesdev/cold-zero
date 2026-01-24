use crate::ir::ir_press_button;
use crate::ir::timings::{COOLER_BTN, MODE_BTN, POWER_BTN, WARMER_BTN};
use core::any::Any;
use core::sync::atomic::AtomicU8;
use flipperzero::{println, warn};
use flipperzero_sys::FuriMutex;

pub struct AppState {
    pub last_called_day: AtomicU8,
    pub heater_state: HeaterState,
    pub mutex: *mut FuriMutex,
}

pub struct HeaterState {
    // Max 35 min 5
    pub temperature: u8,
    pub mode: HeaterMode,
    pub is_on: bool,
}

impl Default for HeaterState {
    fn default() -> Self {
        HeaterState {
            temperature: 25,
            mode: HeaterMode::Eco,
            is_on: false,
        }
    }
}

impl HeaterState {
    pub fn power_on(&mut self) {
        ir_press_button(&POWER_BTN);

        self.temperature = 25;
        self.mode = HeaterMode::Eco;
        self.is_on = true;
    }

    pub fn power_off(&mut self) {
        ir_press_button(&POWER_BTN);

        self.temperature = 25;
        self.mode = HeaterMode::Eco;
        self.is_on = false;
    }

    pub fn change_mode(&mut self, desired_mode: HeaterMode) {
        assert!(self.is_on, "The heater must be on!");

        while self.mode != desired_mode {
            ir_press_button(&MODE_BTN);
            self.mode = self.mode.next();
        }
    }

    pub fn set_temp(&mut self, desired_temp: u8) {
        assert!((5..=35).contains(&desired_temp), "Temperature must be between 5 and 35!");
        assert!(self.is_on, "The heater must be on!");

        if self.temperature == desired_temp {
            warn!("Temperature already at the desired number");
            return;
        }

        let change_needed = (desired_temp - self.temperature) as i8;
        let step = if change_needed > 0 { 1 } else { -1 };
        let button = if step == 1 { &WARMER_BTN } else { &COOLER_BTN };

        for _ in 0..change_needed.abs() {
            ir_press_button(button);
            self.temperature = (self.temperature as i8 + step) as u8;
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum HeaterMode {
    HeatLow = 0,
    HeatHigh,
    Eco
}

impl HeaterMode {
    fn next(&self) -> HeaterMode {
        match self {
            HeaterMode::HeatLow => HeaterMode::HeatHigh,
            HeaterMode::HeatHigh => HeaterMode::Eco,
            HeaterMode::Eco => HeaterMode::HeatLow
        }
    }
}
