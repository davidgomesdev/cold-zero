//! The air conditioner screen: a cursor over the Daikin state.
//!
//! The remote is stateful — one press retransmits everything — so every edit
//! here sends the whole frame straight away, exactly like the physical remote
//! does. See [`crate::daikin`] for the wire format.

use crate::daikin::{Daikin, Fan, MAX_TEMP, MIN_TEMP, Mode, STATE_LEN};
use crate::icons;
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
    Run,
    Quiet,
    Comfort,
    Presence,
    Clean,
}

pub const FIELDS: [Field; 10] = [
    Field::Mode,
    Field::Temp,
    Field::Fan,
    // The ones that get used often enough to want them near the top.
    Field::Comfort,
    Field::Clean,
    Field::Quiet,
    Field::Run,
    Field::SwingV,
    Field::SwingH,
    Field::Presence,
];

impl Field {
    pub fn label(self) -> &'static CStr {
        match self {
            Field::Mode => c"Mode",
            Field::Temp => c"Temp",
            Field::Fan => c"Fan",
            Field::SwingV => c"Swing V",
            Field::SwingH => c"Swing H",
            Field::Run => c"Run",
            Field::Quiet => c"Quiet",
            Field::Comfort => c"Comfort",
            // `daikin` keeps IRremoteESP8266's name for it (Sensor); the
            // remote calls it Intelligent Eye, and what it actually does is
            // notice whether anyone is in the room.
            Field::Presence => c"Presence",
            // `daikin` keeps IRremoteESP8266's name for it (Mould); "Clean"
            // is what the row says, because the option is the internal-dry
            // run after shutdown, not anything about mould you can see.
            Field::Clean => c"Clean",
        }
    }

    /// Rows whose values are named rather than numeric or on/off. They open a
    /// picker; everything else changes in place.
    pub fn picker(self) -> Option<Picker> {
        match self {
            Field::Mode => Some(Picker::Mode),
            Field::Run => Some(Picker::Run),
            _ => None,
        }
    }
}

/// How hard the unit is allowed to run. Eco caps its draw and Powerful lifts
/// it, and the protocol won't hold both — `set_powerful` clears econo and
/// `set_econo` clears powerful. Two rows that silently switch each other off
/// is worse than one row with three values, so this is one row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    Normal,
    Eco,
    Powerful,
}

/// The order each picker lays its icons out, top to bottom.
pub const MODES: [Mode; 5] = [Mode::Auto, Mode::Cool, Mode::Heat, Mode::Dry, Mode::Fan];
pub const RUNS: [Run; 3] = [Run::Normal, Run::Eco, Run::Powerful];

/// The rows that open a full-screen list of icons instead of changing in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picker {
    Mode,
    Run,
}

impl Picker {
    pub fn title(self) -> &'static CStr {
        match self {
            Picker::Mode => c"Mode",
            Picker::Run => c"Run",
        }
    }

    pub fn len(self) -> usize {
        match self {
            Picker::Mode => MODES.len(),
            Picker::Run => RUNS.len(),
        }
    }

    /// The icon and name for one row of the picker.
    pub fn option(self, index: usize) -> (&'static [u8; 32], &'static CStr) {
        match self {
            Picker::Mode => match MODES[index] {
                Mode::Auto => (&icons::MODE_AUTO, c"Auto"),
                Mode::Cool => (&icons::MODE_COOL, c"Cool"),
                Mode::Heat => (&icons::MODE_HEAT, c"Heat"),
                Mode::Dry => (&icons::MODE_DRY, c"Dry"),
                // The tower fan's icon does for fan-only mode too.
                Mode::Fan => (&icons::FAN, c"Fan"),
            },
            Picker::Run => match RUNS[index] {
                Run::Normal => (&icons::RUN_NORMAL, c"Normal"),
                Run::Eco => (&icons::RUN_ECO, c"Eco"),
                // `daikin` keeps IRremoteESP8266's name for it (Powerful).
                Run::Powerful => (&icons::RUN_POWER, c"Power"),
            },
        }
    }

    /// Where the picker opens: on whatever is in effect now.
    pub fn current(self, ac: &Daikin) -> usize {
        match self {
            Picker::Mode => MODES.iter().position(|m| *m == ac.mode()).unwrap_or(0),
            Picker::Run => {
                // Powerful wins the read-back: the protocol can't hold both, so
                // if its bit is set the econo bit is already clear.
                let run = if ac.powerful() {
                    Run::Powerful
                } else if ac.econo() {
                    Run::Eco
                } else {
                    Run::Normal
                };
                RUNS.iter().position(|r| *r == run).unwrap_or(0)
            }
        }
    }

    fn apply(self, ac: &mut Daikin, index: usize) {
        match self {
            Picker::Mode => ac.set_mode(MODES[index]),
            Picker::Run => match RUNS[index] {
                Run::Normal => {
                    ac.set_powerful(false);
                    ac.set_econo(false);
                }
                // Each setter already clears whatever it excludes.
                Run::Eco => ac.set_econo(true),
                Run::Powerful => ac.set_powerful(true),
            },
        }
    }
}

pub struct AcState {
    pub daikin: Daikin,
    pub field: Field,
    /// The open picker and where its cursor sits, or `None` while none is open.
    pub menu: Option<(Picker, usize)>,
}

impl Default for AcState {
    fn default() -> Self {
        AcState {
            daikin: Daikin::default(),
            field: Field::Mode,
            menu: None,
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
        match self.menu {
            // In a picker only a tap on OK commits. Holding it would
            // otherwise fall through to the power toggle.
            Some(_) => key == InputKeyOk && type_ == InputTypeShort,
            None => match key {
                // Tap toggles power, hold resends; both transmit everything.
                InputKeyOk => type_ == InputTypeShort || type_ == InputTypeLong,
                // A picker row opens rather than transmitting.
                InputKeyLeft | InputKeyRight => {
                    self.field.picker().is_none() && type_ == InputTypeShort
                }
                _ => false,
            },
        }
    }

    /// The presses that only move a cursor around.
    #[allow(non_upper_case_globals)]
    pub fn navigate(&mut self, key: InputKey) {
        if self.menu.is_some() {
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
            // Only a picker row gets here; `sends` routed the others away.
            InputKeyLeft | InputKeyRight => {
                if let Some(picker) = self.field.picker() {
                    self.menu = Some((picker, picker.current(&self.daikin)));
                }
            }
            _ => {}
        }
    }

    /// Back inside a picker closes it rather than leaving the screen.
    /// Returns whether there was a picker to close.
    pub fn close_menu(&mut self) -> bool {
        self.menu.take().is_some()
    }

    fn menu_step(&mut self, down: bool) {
        let Some((picker, index)) = self.menu else { return };
        let index = if down {
            (index + 1) % picker.len()
        } else {
            (index + picker.len() - 1) % picker.len()
        };
        self.menu = Some((picker, index));
    }

    /// OK in a picker: apply the highlighted option, close, transmit.
    pub fn commit_menu(&mut self) {
        let Some((picker, index)) = self.menu.take() else {
            return;
        };
        info!("A/C: picking an option");
        picker.apply(&mut self.daikin, index);
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
            // Handled by a picker; `sends` never routes these here.
            Field::Mode | Field::Run => return,
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
            Field::Quiet => ac.set_quiet(!ac.quiet()),
            Field::Comfort => ac.set_comfort(!ac.comfort()),
            Field::Presence => ac.set_sensor(!ac.sensor()),
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

    /// What a picker row shows on the list: the option currently in effect.
    pub fn picker_label(&self, picker: Picker) -> &'static CStr {
        picker.option(picker.current(&self.daikin)).1
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
            Field::Quiet => Some(ac.quiet()),
            Field::Comfort => Some(ac.comfort()),
            Field::Presence => Some(ac.sensor()),
            Field::Clean => Some(ac.mold()),
            _ => None,
        }
    }
}
