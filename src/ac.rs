//! The air conditioner screen: a cursor over the Daikin state.
//!
//! The remote is stateful — one press retransmits everything — so every edit
//! here sends the whole frame straight away, exactly like the physical remote
//! does. See [`crate::daikin`] for the wire format.

use crate::daikin::{Daikin, Fan, MAX_TEMP, MIN_TEMP, Mode, STATE_LEN};
use core::ffi::CStr;
use flipperzero::furi::hal::rtc::datetime;
use flipperzero::io::{Read, Write};
use flipperzero::storage::{File, Storage};
use flipperzero::{info, warn};
use flipperzero_sys::{
    InputKey, InputKeyDown, InputKeyLeft, InputKeyOk, InputKeyRight, InputKeyUp, InputType,
    InputTypeLong, InputTypeShort, storage_common_mkdir,
};

/// Where the remembered state lives. `/ext/apps_data` is always there; the
/// per-app directory under it may not be, so saving creates it.
const STATE_DIR: &CStr = c"/ext/apps_data/cold-zero";
const STATE_PATH: &CStr = c"/ext/apps_data/cold-zero/ac.bin";

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
    Clean,
}

pub const FIELDS: [Field; 11] = [
    Field::Mode,
    Field::Temp,
    Field::Fan,
    // The three that get used often enough to want them near the top.
    Field::Comfort,
    Field::Clean,
    Field::Quiet,
    Field::SwingV,
    Field::SwingH,
    Field::Powerful,
    Field::Econo,
    Field::Sensor,
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
            // `daikin` keeps IRremoteESP8266's name for it (Mould); "Clean"
            // is what the row says, because the option is the internal-dry
            // run after shutdown, not anything about mould you can see.
            Field::Clean => c"Clean",
        }
    }
}

/// The order the mode picker lays its icons out, top to bottom.
pub const MODES: [Mode; 5] = [Mode::Auto, Mode::Cool, Mode::Heat, Mode::Dry, Mode::Fan];

pub struct AcState {
    pub daikin: Daikin,
    pub field: Field,
    /// The cursor inside the mode picker, or `None` while it is closed. Mode is
    /// the one setting with five named values rather than a number or a flag,
    /// so it gets a picker with icons instead of a blind left/right cycle.
    pub mode_menu: Option<Mode>,
}

impl Default for AcState {
    fn default() -> Self {
        AcState {
            daikin: Daikin::default(),
            field: Field::Mode,
            mode_menu: None,
        }
    }
}

impl AcState {
    /// Pick up where the last session left off.
    ///
    /// The A/C never reports back, so the app's state is a belief, and starting
    /// every launch from a hardcoded default threw that belief away — the first
    /// keypress would then push a stale guess (including power) onto a unit
    /// that was set up quite differently. Remembering it is only right while
    /// the Flipper is the only thing touching the A/C; the physical remote
    /// still desyncs it, and `Hold OK` is still the way back.
    pub fn load() -> Self {
        let mut state = AcState::default();

        let mut raw = [0u8; STATE_LEN];
        match File::open(STATE_PATH).and_then(|mut file| file.read(&mut raw)) {
            // A short read means a truncated file, so treat it like corruption.
            Ok(STATE_LEN) => match Daikin::from_raw(raw) {
                Some(daikin) => {
                    info!("A/C: restored the saved state");
                    state.daikin = daikin;
                }
                None => warn!("A/C: saved state is corrupt, starting fresh"),
            },
            Ok(read) => warn!("A/C: saved state is {} bytes, starting fresh", read),
            // Overwhelmingly just "no file yet", i.e. the first ever launch.
            Err(_) => info!("A/C: no saved state, starting fresh"),
        }

        state
    }

    /// Best-effort: a Flipper with no SD card still has to work, so a failure
    /// here is logged and dropped rather than propagated.
    fn save(&self) {
        unsafe { storage_common_mkdir(Storage::open().as_ptr(), STATE_DIR.as_ptr()) };

        let saved = File::create(STATE_PATH).and_then(|mut file| {
            file.write_all(self.daikin.raw())?;
            file.flush()
        });

        if saved.is_err() {
            warn!("A/C: could not save the state");
        }
    }

    /// Whether a press will put a frame on the air. The caller needs to know
    /// *before* dispatching it, because a send blocks for the whole frame and
    /// the screen has to paint "Changing..." first. Keeping the answer next to
    /// the handlers is what stops the two drifting apart.
    #[allow(non_upper_case_globals)]
    pub fn sends(&self, key: InputKey, type_: InputType) -> bool {
        match self.mode_menu {
            // In the picker only a tap on OK commits. Holding it would
            // otherwise fall through to the power toggle.
            Some(_) => key == InputKeyOk && type_ == InputTypeShort,
            None => match key {
                // Tap toggles power, hold resends; both transmit everything.
                InputKeyOk => type_ == InputTypeShort || type_ == InputTypeLong,
                // The Mode row opens the picker rather than transmitting.
                InputKeyLeft | InputKeyRight => {
                    self.field != Field::Mode && type_ == InputTypeShort
                }
                _ => false,
            },
        }
    }

    /// The presses that only move a cursor around.
    #[allow(non_upper_case_globals)]
    pub fn navigate(&mut self, key: InputKey) {
        if self.mode_menu.is_some() {
            match key {
                InputKeyUp => self.menu_step(false),
                InputKeyDown => self.menu_step(true),
                _ => {}
            }
            return;
        }

        match key {
            InputKeyUp => self.move_cursor(false),
            InputKeyDown => self.move_cursor(true),
            // Only the Mode row gets here; `sends` routed the other rows away.
            InputKeyLeft | InputKeyRight => self.mode_menu = Some(self.daikin.mode()),
            _ => {}
        }
    }

    /// Back inside the picker closes it rather than leaving the screen.
    /// Returns whether there was a picker to close.
    pub fn close_menu(&mut self) -> bool {
        self.mode_menu.take().is_some()
    }

    fn menu_step(&mut self, down: bool) {
        let Some(mode) = self.mode_menu else { return };
        let index = MODES.iter().position(|m| *m == mode).unwrap_or(0);
        let index = if down {
            (index + 1) % MODES.len()
        } else {
            (index + MODES.len() - 1) % MODES.len()
        };
        self.mode_menu = Some(MODES[index]);
    }

    /// OK in the picker: take the highlighted mode, close, transmit.
    pub fn commit_mode(&mut self) {
        let Some(mode) = self.mode_menu.take() else {
            return;
        };
        info!("A/C: picking a mode");
        self.daikin.set_mode(mode);
        self.send();
    }

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
            // Handled by the picker; `sends` never routes it here.
            Field::Mode => return,
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
            Field::Clean => ac.set_mold(!ac.mold()),
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
        // Saving after the send keeps the blocking file write off the path
        // between the keypress and the A/C reacting.
        self.save();
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
            Field::Clean => Some(ac.mold()),
            _ => None,
        }
    }
}
