use crate::ir::fan as fan_ir;
use crate::ir::ir_press_button;
use flipperzero::info;
use ufmt::derive::uDebug;

pub struct FanState {
    pub is_on: bool,
    /// 0–9 hours
    pub timer: u8,
    /// Whether TIMER has been pressed since power on — the fan swallows the
    /// first one, so it has to be sent twice to register.
    timer_pressed: bool,
    pub light: FanLight,
    pub mode: FanMode,
    pub speed: FanSpeed,
    pub rotating: bool,
}

#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq, uDebug)]
pub enum FanLight {
    Full,
    Partial,
    Off,
}

/// Set by the MODE button, independent of speed.
#[derive(Debug, PartialEq, Eq, uDebug)]
pub enum FanMode {
    Normal,
    Sleep,
    Nature,
}

/// Set by the SPEED button. Every mode has F1–F3; SF (steady flow) exists
/// only in Normal.
#[derive(Debug, PartialEq, Eq, uDebug)]
pub enum FanSpeed {
    F1,
    F2,
    F3,
    SF,
}

impl FanMode {
    fn next(&self) -> FanMode {
        match self {
            FanMode::Normal => FanMode::Sleep,
            FanMode::Sleep => FanMode::Nature,
            FanMode::Nature => FanMode::Normal,
        }
    }
}

impl FanSpeed {
    fn next(&self, mode: &FanMode) -> FanSpeed {
        match self {
            FanSpeed::F1 => FanSpeed::F2,
            FanSpeed::F2 => FanSpeed::F3,
            // SF is only reachable in Normal; the other modes wrap at F3
            FanSpeed::F3 if *mode == FanMode::Normal => FanSpeed::SF,
            _ => FanSpeed::F1,
        }
    }
}

impl Default for FanState {
    fn default() -> Self {
        FanState {
            is_on: false,
            timer: 0,
            timer_pressed: false,
            light: FanLight::Full,
            mode: FanMode::Normal,
            speed: FanSpeed::F2,
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
        self.mode = FanMode::Normal;
        self.speed = FanSpeed::F2;
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
        info!("Fan: next speed (from {:?})", self.speed);
        ir_press_button(&fan_ir::SPEED_BTN);

        self.speed = self.speed.next(&self.mode);
        self.turn_light_off();
    }

    pub fn next_mode(&mut self) {
        info!("Fan: next mode (from {:?})", self.mode);
        ir_press_button(&fan_ir::MODE_BTN);

        self.mode = self.mode.next();
        // SF doesn't exist outside Normal, so leaving Normal drops to F1
        if self.mode != FanMode::Normal && self.speed == FanSpeed::SF {
            self.speed = FanSpeed::F1;
        }

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
