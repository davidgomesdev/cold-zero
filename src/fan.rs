use crate::ir::fan as fan_ir;
use crate::ir::ir_press_button;
use flipperzero::info;
use ufmt::derive::uDebug;

pub struct FanState {
    pub is_on: bool,
    /// 0–9 hours
    pub timer: u8,
    pub light: FanLight,
    pub fan_mode: FanMode,
}

#[derive(Debug, PartialEq, Eq, uDebug)]
pub enum FanLight {
    Full,
    Partial,
    Off,
}

#[derive(Debug, PartialEq, Eq, uDebug)]
pub enum FanMode {
    F1,
    F2,
    F3,
    Sleep,
    Nature,
}

impl Default for FanState {
    fn default() -> Self {
        FanState {
            is_on: false,
            timer: 0,
            light: FanLight::Full,
            fan_mode: FanMode::F2,
        }
    }
}

impl FanState {
    pub fn power_on(&mut self) {
        info!("Fan: powering on");
        ir_press_button(&fan_ir::POWER_BTN);
        // First TIMER press is ignored (same physical quirk as heater's warmer button)
        ir_press_button(&fan_ir::TIMER_BTN);
        ir_press_button(&fan_ir::TIMER_BTN);
        // LIGHT must come last — any other button press reactivates the light
        ir_press_button(&fan_ir::LIGHT_BTN); // Full -> Partial
        ir_press_button(&fan_ir::LIGHT_BTN); // Partial -> Off

        self.is_on = true;
        self.timer = 1;
        self.light = FanLight::Off;
        // fan_mode stays F2 (fan defaults to F2 on power on)
    }

    pub fn power_off(&mut self) {
        info!("Fan: powering off");
        ir_press_button(&fan_ir::POWER_BTN);

        self.is_on = false;
        self.timer = 0;
        self.light = FanLight::Full;
    }
}
