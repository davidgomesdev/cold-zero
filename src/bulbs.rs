//! Two LED bulbs, driven over GPIO rather than IR.
//!
//! The bulbs are Zigbee/Matter and live on Home Assistant, so the Flipper can't
//! talk to them directly. Instead each bulb has a wire to an ESP8266 running
//! ESPHome: PA7 -> GPIO5 and PA6 -> GPIO4, with the Flipper's GND tied to the
//! board's. Each ESP pin is an `INPUT_PULLUP` + `inverted` binary sensor whose
//! `on_press` calls `light.toggle`, so pulling the line low for a moment is one
//! button press. PA7 is the escritorio, PA6 the quarto. The ESP holds the
//! line high on its own, which means an
//! unplugged cable or a closed app reads as released, never stuck-pressed.

use core::time::Duration;
use flipperzero::furi::thread::sleep;
use flipperzero::info;
use flipperzero_sys::{
    GpioModeAnalog, GpioModeOutputPushPull, GpioPin, GpioPullNo, GpioSpeedLow, furi_hal_gpio_init,
    furi_hal_gpio_write, gpio_ext_pa6, gpio_ext_pa7,
};

/// Long enough to clear the 20ms `delayed_on_off` debounce on the ESP side.
const PULSE: Duration = Duration::from_millis(50);

#[derive(Default)]
pub struct BulbsState {
    /// Assumed state only — the Flipper sends toggles and never hears back, so
    /// changing a bulb from Home Assistant desyncs this until the next press.
    /// Same open-loop deal as the heater and fan.
    pub escritorio: bool,
    pub quarto: bool,
}

fn pin(bulb: Bulb) -> *const GpioPin {
    match bulb {
        Bulb::Escritorio => &raw const gpio_ext_pa7,
        Bulb::Quarto => &raw const gpio_ext_pa6,
    }
}

#[derive(Clone, Copy)]
enum Bulb {
    Escritorio,
    Quarto,
}

/// Park both lines high *before* making them outputs. The output register
/// powers up low, so initialising first would drive a falling edge and the ESP
/// would register a phantom press the moment the app opens.
pub fn init() {
    unsafe {
        for bulb in [Bulb::Escritorio, Bulb::Quarto] {
            furi_hal_gpio_write(pin(bulb), true);
            furi_hal_gpio_init(pin(bulb), GpioModeOutputPushPull, GpioPullNo, GpioSpeedLow);
        }
    }
}

/// Hand the pins back on exit; the ESP's pullup holds them released.
pub fn deinit() {
    unsafe {
        for bulb in [Bulb::Escritorio, Bulb::Quarto] {
            furi_hal_gpio_init(pin(bulb), GpioModeAnalog, GpioPullNo, GpioSpeedLow);
        }
    }
}

fn press(bulb: Bulb) {
    unsafe {
        furi_hal_gpio_write(pin(bulb), false);
        sleep(PULSE);
        furi_hal_gpio_write(pin(bulb), true);
    }
}

impl BulbsState {
    pub fn toggle_escritorio(&mut self) {
        info!("Bulbs: toggling escritorio");
        press(Bulb::Escritorio);
        self.escritorio = !self.escritorio;
    }

    pub fn toggle_quarto(&mut self) {
        info!("Bulbs: toggling quarto");
        press(Bulb::Quarto);
        self.quarto = !self.quarto;
    }

    /// OK drives both to the same state, so one press turns the pair on and the
    /// next turns the pair off even if they'd drifted apart.
    pub fn set_both(&mut self, on: bool) {
        info!("Bulbs: setting both");
        if self.escritorio != on {
            self.toggle_escritorio();
        }
        if self.quarto != on {
            self.toggle_quarto();
        }
    }

    pub fn both_on(&self) -> bool {
        self.escritorio && self.quarto
    }
}
