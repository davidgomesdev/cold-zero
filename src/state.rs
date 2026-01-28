use crate::ir::ir_press_button;
use crate::ir::timings::{COOLER_BTN, MODE_BTN, POWER_BTN, WARMER_BTN};
use flipperzero::{info, warn};
use flipperzero_sys::FuriMutex;
use ufmt::derive::uDebug;

pub struct AppState {
    pub last_called_day: u8,
    pub heater_state: HeaterState,
    pub mutex: *mut FuriMutex,
}

pub struct HeaterState {
    /// Temperature range: 5°C (min) to 35°C (max)
    pub temperature: u8,
    pub mode: HeaterMode,
    pub is_on: bool,
}

impl Default for HeaterState {
    fn default() -> Self {
        HeaterState {
            temperature: 23,
            mode: HeaterMode::Eco,
            is_on: false,
        }
    }
}

impl HeaterState {
    pub fn power_on(&mut self) {
        info!("Powering on");

        ir_press_button(&POWER_BTN);

        self.temperature = 23;
        self.mode = HeaterMode::Eco;
        self.is_on = true;
    }

    pub fn power_off(&mut self) {
        info!("Powering off");

        ir_press_button(&POWER_BTN);

        self.temperature = 23;
        self.mode = HeaterMode::Eco;
        self.is_on = false;
    }

    pub fn change_mode(&mut self, desired_mode: HeaterMode) {
        assert!(self.is_on, "The heater must be on!");

        info!("Changing mode to {:?}", desired_mode);

        while self.mode != desired_mode {
            ir_press_button(&MODE_BTN);
            self.mode = self.mode.next();
        }
    }

    pub fn set_temp(&mut self, desired_temp: u8) {
        assert!(
            (5..=35).contains(&desired_temp),
            "Temperature must be between 5 and 35!"
        );
        assert!(self.is_on, "The heater must be on!");

        if self.temperature == desired_temp {
            warn!("Temperature already at the desired number");
            return;
        }

        info!(
            "Setting temp to {} (from {})",
            desired_temp, self.temperature
        );

        let change_needed = desired_temp as i8 - self.temperature as i8;
        let button = if change_needed.is_positive() {
            &WARMER_BTN
        } else {
            &COOLER_BTN
        };

        // The +1 is because, for some reason, the first warm button press doesn't register
        // (even with the remote)
        for _ in 0..change_needed.abs() + 1 {
            ir_press_button(button);
        }

        self.temperature = desired_temp;
    }
}

#[derive(Debug, PartialEq, Eq, uDebug)]
pub enum HeaterMode {
    HeatLow = 0,
    HeatHigh,
    Eco,
}

impl HeaterMode {
    fn next(&self) -> HeaterMode {
        match self {
            HeaterMode::HeatLow => HeaterMode::HeatHigh,
            HeaterMode::HeatHigh => HeaterMode::Eco,
            HeaterMode::Eco => HeaterMode::HeatLow,
        }
    }
}
