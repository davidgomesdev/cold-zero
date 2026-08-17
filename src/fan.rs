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
    pub rotating: bool,
}

#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq, uDebug)]
pub enum FanLight {
    Full,
    Partial,
    Off,
}

#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq, uDebug)]
pub enum FanMode {
    F1,
    F2,
    F3,
    Sleep,
    Nature,
}

impl FanMode {
    /// The SPEED button only cycles the three fan speeds; Sleep/Nature come
    /// from the MODE button and drop back to F1 when speed is pressed.
    fn next_speed(&self) -> FanMode {
        match self {
            FanMode::F1 => FanMode::F2,
            FanMode::F2 => FanMode::F3,
            _ => FanMode::F1,
        }
    }
}

impl Default for FanState {
    fn default() -> Self {
        FanState {
            is_on: false,
            timer: 0,
            light: FanLight::Full,
            fan_mode: FanMode::F2,
            rotating: false,
        }
    }
}

impl FanState {
    pub fn power_on(&mut self) {
        info!("Fan: powering on");
        ir_press_button(&fan_ir::POWER_BTN);

        self.is_on = true;
        self.timer = 0;
        self.light = FanLight::Full;
        self.rotating = false;
        // fan_mode stays F2 (fan defaults to F2 on power on)
    }

    /// Power on and apply the usual setup: 1h timer and light off.
    pub fn power_on_full(&mut self) {
        self.power_on();

        // First TIMER press is ignored (same physical quirk as heater's warmer button)
        ir_press_button(&fan_ir::TIMER_BTN);
        ir_press_button(&fan_ir::TIMER_BTN);
        self.timer = 1;

        self.turn_light_off();
    }

    pub fn power_off(&mut self) {
        info!("Fan: powering off");
        ir_press_button(&fan_ir::POWER_BTN);

        self.is_on = false;
        self.timer = 0;
        self.light = FanLight::Full;
        self.rotating = false;
    }

    pub fn rotate(&mut self) {
        info!("Fan: toggling rotation");
        ir_press_button(&fan_ir::ROTATE_BTN);

        self.rotating = !self.rotating;
        self.turn_light_off();
    }

    pub fn next_speed(&mut self) {
        info!("Fan: next speed (from {:?})", self.fan_mode);
        ir_press_button(&fan_ir::SPEED_BTN);

        self.fan_mode = self.fan_mode.next_speed();
        self.turn_light_off();
    }

    /// Any other button press reactivates the light (back to Full), so two
    /// presses always land on Off.
    fn turn_light_off(&mut self) {
        ir_press_button(&fan_ir::LIGHT_BTN); // Full -> Partial
        ir_press_button(&fan_ir::LIGHT_BTN); // Partial -> Off

        self.light = FanLight::Off;
    }
}
