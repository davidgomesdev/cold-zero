//! The air conditioner screen: a cursor over the Daikin state.
//!
//! The remote is stateful — one press retransmits everything — so every edit
//! here sends the whole frame straight away, exactly like the physical remote
//! does. See [`crate::daikin`] for the wire format.

use crate::daikin::{Daikin, Fan, MAX_TEMP, MIN_TEMP, Mode};
use core::ffi::CStr;
use flipperzero::furi::hal::rtc::datetime;
use flipperzero::info;

/// The editable rows, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Mode,
    Temp,
    Fan,
    SwingV,
    SwingH,
    Powerful,
    Quiet,
    Econo,
    Comfort,
    Sensor,
    Mold,
}

pub const FIELDS: [Field; 11] = [
    Field::Mode,
    Field::Temp,
    Field::Fan,
    Field::SwingV,
    Field::SwingH,
    Field::Powerful,
    Field::Quiet,
    Field::Econo,
    Field::Comfort,
    Field::Sensor,
    Field::Mold,
];

impl Field {
    pub fn label(self) -> &'static CStr {
        match self {
            Field::Mode => c"Mode",
            Field::Temp => c"Temp",
            Field::Fan => c"Fan",
            Field::SwingV => c"Swing V",
            Field::SwingH => c"Swing H",
            Field::Powerful => c"Powerful",
            Field::Quiet => c"Quiet",
            Field::Econo => c"Econo",
            Field::Comfort => c"Comfort",
            Field::Sensor => c"Eye",
            Field::Mold => c"Mold",
        }
    }
}

pub struct AcState {
    pub daikin: Daikin,
    pub field: Field,
}

impl Default for AcState {
    fn default() -> Self {
        AcState {
            daikin: Daikin::default(),
            field: Field::Mode,
        }
    }
}

impl AcState {
    /// Up/Down walk the rows. No wrap-around: with eleven rows on one screen,
    /// stopping at the ends is less surprising than jumping across.
    pub fn move_cursor(&mut self, down: bool) {
        let index = FIELDS.iter().position(|f| *f == self.field).unwrap_or(0);
        let index = if down {
            (index + 1).min(FIELDS.len() - 1)
        } else {
            index.saturating_sub(1)
        };
        self.field = FIELDS[index];
    }

    /// Left/Right change the selected row and put the new state on the air.
    pub fn adjust(&mut self, forward: bool) {
        info!("A/C: adjusting a setting");

        let ac = &mut self.daikin;
        match self.field {
            Field::Mode => {
                let mode = if forward {
                    ac.mode().next()
                } else {
                    ac.mode().prev()
                };
                ac.set_mode(mode);
            }
            Field::Temp => {
                let temp = if forward {
                    (ac.temp() + 1).min(MAX_TEMP)
                } else {
                    ac.temp().saturating_sub(1).max(MIN_TEMP)
                };
                ac.set_temp(temp);
            }
            Field::Fan => {
                let fan = if forward {
                    ac.fan().next()
                } else {
                    ac.fan().prev()
                };
                ac.set_fan(fan);
            }
            // The rest are flags, so either direction is just a toggle.
            Field::SwingV => ac.set_swing_vertical(!ac.swing_vertical()),
            Field::SwingH => ac.set_swing_horizontal(!ac.swing_horizontal()),
            Field::Powerful => ac.set_powerful(!ac.powerful()),
            Field::Quiet => ac.set_quiet(!ac.quiet()),
            Field::Econo => ac.set_econo(!ac.econo()),
            Field::Comfort => ac.set_comfort(!ac.comfort()),
            Field::Sensor => ac.set_sensor(!ac.sensor()),
            Field::Mold => ac.set_mold(!ac.mold()),
        }

        self.send();
    }

    pub fn toggle_power(&mut self) {
        info!("A/C: toggling power");
        let on = self.daikin.power();
        self.daikin.set_power(!on);
        self.send();
    }

    /// Stamp the frame with the Flipper's clock — the real remote does, and the
    /// A/C's own timers run off it — then transmit.
    pub fn send(&mut self) {
        let time = datetime();
        self.daikin
            .set_current_time(time.hour as u16 * 60 + time.minute as u16);
        // Furi counts Monday as 1; the remote counts Sunday as 1.
        self.daikin.set_current_day(time.weekday % 7 + 1);
        self.daikin.send();
    }

    pub fn mode_label(&self) -> &'static CStr {
        match self.daikin.mode() {
            Mode::Auto => c"Auto",
            Mode::Cool => c"Cool",
            Mode::Heat => c"Heat",
            Mode::Dry => c"Dry",
            Mode::Fan => c"Fan",
        }
    }

    pub fn fan_label(&self) -> &'static CStr {
        match self.daikin.fan() {
            Fan::Auto => c"Auto",
            Fan::Quiet => c"Quiet",
            Fan::F1 => c"1",
            Fan::F2 => c"2",
            Fan::F3 => c"3",
            Fan::F4 => c"4",
            Fan::F5 => c"5",
        }
    }

    /// The flag rows all render the same way; `None` means the row shows
    /// something other than On/Off.
    pub fn flag(&self, field: Field) -> Option<bool> {
        let ac = &self.daikin;
        match field {
            Field::SwingV => Some(ac.swing_vertical()),
            Field::SwingH => Some(ac.swing_horizontal()),
            Field::Powerful => Some(ac.powerful()),
            Field::Quiet => Some(ac.quiet()),
            Field::Econo => Some(ac.econo()),
            Field::Comfort => Some(ac.comfort()),
            Field::Sensor => Some(ac.sensor()),
            Field::Mold => Some(ac.mold()),
            _ => None,
        }
    }
}
