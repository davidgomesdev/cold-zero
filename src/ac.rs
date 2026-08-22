//! The air conditioner screen: a cursor over the Daikin state.
//!
//! The remote is stateful — one press retransmits everything — so every edit
//! here sends the whole frame straight away, exactly like the physical remote
//! does. See [`crate::daikin`] for the wire format.

use crate::daikin::{self, Daikin, Fan, MAX_TEMP, MIN_TEMP, Mode, STATE_LEN};
use crate::icons;
use core::ffi::CStr;
use flipperzero::furi::hal::rtc::datetime;
use flipperzero::io::{Read, Write};
use flipperzero::storage::{File, Storage};
use flipperzero::{info, warn};
use flipperzero_sys::{
    InputKey, InputKeyDown, InputKeyLeft, InputKeyOk, InputKeyRight, InputKeyUp, InputType,
    InputTypeLong, InputTypePress, InputTypeRelease, InputTypeRepeat, InputTypeShort,
    storage_common_mkdir,
};

/// Minutes past midnight, right now. The A/C's timers run off the clock every
/// frame carries, so this is both what gets stamped and what timers are
/// measured from.
pub fn minutes_now() -> u16 {
    let time = datetime();
    time.hour as u16 * 60 + time.minute as u16
}

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
    Timer,
    OffAt,
    OnAt,
    TimerClear,
    SwingV,
    SwingH,
    Run,
    Quiet,
    Comfort,
    Presence,
    Clean,
}

/// The main list.
pub const FIELDS: [Field; 11] = [
    Field::Mode,
    Field::Temp,
    Field::Fan,
    Field::Timer,
    // The ones that get used often enough to want them near the top.
    Field::Comfort,
    Field::Clean,
    Field::Quiet,
    Field::Run,
    Field::SwingV,
    Field::SwingH,
    Field::Presence,
];

/// The Timer row's own list. Two independent times is one too many for a row,
/// and neither is a choice from a set, so this is a sub-list rather than a
/// [`Picker`].
pub const TIMER_FIELDS: [Field; 3] = [Field::OffAt, Field::OnAt, Field::TimerClear];

/// Which list the A/C screen is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Settings,
    Timers,
}

impl View {
    pub fn fields(self) -> &'static [Field] {
        match self {
            View::Settings => &FIELDS,
            View::Timers => &TIMER_FIELDS,
        }
    }
}

impl Field {
    pub fn label(self) -> &'static CStr {
        match self {
            Field::Mode => c"Mode",
            Field::Temp => c"Temp",
            Field::Fan => c"Fan",
            Field::Timer => c"Timer",
            Field::OffAt => c"Off at",
            Field::OnAt => c"On at",
            Field::TimerClear => c"Clear",
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

    /// Rows worth holding an arrow on. A flag has nothing to run through --
    /// repeating it would just toggle at 10Hz and land wherever the release
    /// happened to fall.
    pub fn repeats(self) -> bool {
        matches!(self, Field::Temp | Field::Fan | Field::OffAt | Field::OnAt)
    }

    /// Rows that lead somewhere instead of changing in place. They get a ">"
    /// on the list and swallow Back on the way out.
    pub fn opens(self) -> bool {
        self.picker().is_some() || self == Field::Timer
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
    /// Which list is showing. The cursor lives in `field`, which always points
    /// into `view.fields()`.
    pub view: View,
    pub field: Field,
    /// The open picker and where its cursor sits, or `None` while none is open.
    pub menu: Option<(Picker, usize)>,
    /// Set while a held arrow is running through a row's values. The steps are
    /// applied locally and one frame goes out on release: every Daikin frame
    /// carries the whole state, so only the last one matters, and a send blocks
    /// for ~400ms -- transmitting per step would leave the display seconds
    /// behind the key.
    repeating: bool,
}

impl Default for AcState {
    fn default() -> Self {
        AcState {
            daikin: Daikin::default(),
            view: View::Settings,
            field: Field::Mode,
            menu: None,
            repeating: false,
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
                InputKeyLeft | InputKeyRight => {
                    if self.field.opens() {
                        // The row leads somewhere rather than transmitting.
                        false
                    } else if type_ == InputTypeShort {
                        true
                    } else {
                        // The single frame for a whole held run goes out when
                        // the key comes back up.
                        type_ == InputTypeRelease && self.repeating
                    }
                }
                _ => false,
            },
        }
    }

    /// The presses that only move a cursor around.
    #[allow(non_upper_case_globals)]
    pub fn navigate(&mut self, key: InputKey, type_: InputType) {
        // Every press starts here, so it is the one place guaranteed to run
        // before a hold begins. Without it a run abandoned by some other key
        // would leave the flag set and make the next release transmit.
        if type_ == InputTypePress {
            self.repeating = false;
            return;
        }

        if self.menu.is_some() {
            if type_ == InputTypeShort {
                match key {
                    InputKeyUp => self.menu_step(false),
                    InputKeyDown => self.menu_step(true),
                    _ => {}
                }
            }
            return;
        }

        // Walking the list repeats too. A tap emits Short, a hold emits Long
        // then Repeats, so the two never both fire for one press.
        if key == InputKeyUp || key == InputKeyDown {
            if type_ == InputTypeShort || type_ == InputTypeLong || type_ == InputTypeRepeat {
                self.move_cursor(key == InputKeyDown);
            }
            return;
        }

        if key != InputKeyLeft && key != InputKeyRight {
            return;
        }

        if type_ == InputTypeShort {
            // Only a row that opens gets here; `sends` routed the rest away.
            if let Some(picker) = self.field.picker() {
                self.menu = Some((picker, picker.current(&self.daikin)));
            } else if self.field == Field::Timer {
                self.view = View::Timers;
                self.field = TIMER_FIELDS[0];
            }
            return;
        }

        // A held arrow: Long is the first step of the run, Repeat the rest.
        if (type_ == InputTypeLong || type_ == InputTypeRepeat) && self.field.repeats() {
            self.step(key == InputKeyRight);
            self.repeating = true;
        }
    }

    /// Back unwinds one level: a picker first, then the timer sub-list, and
    /// only once both are gone does the screen itself close. Returns whether
    /// this level swallowed the press.
    pub fn go_back(&mut self) -> bool {
        self.repeating = false;

        if self.menu.take().is_some() {
            return true;
        }

        if self.view == View::Timers {
            self.view = View::Settings;
            self.field = Field::Timer;
            return true;
        }

        false
    }

    /// Tap-only, deliberately: these rings are three and five long, so holding
    /// an arrow would spin through them faster than you could stop on one. The
    /// settings list can repeat because it clamps at its ends.
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

    /// Up/Down walk the rows, and repeat while held. No wrap-around: with the
    /// whole list on one screen, stopping at the ends is less surprising than
    /// jumping across, and it is what makes holding Down mean "go to the
    /// bottom" rather than "spin".
    pub fn move_cursor(&mut self, down: bool) {
        let fields = self.view.fields();
        let index = fields.iter().position(|f| *f == self.field).unwrap_or(0);
        let index = if down {
            (index + 1).min(fields.len() - 1)
        } else {
            index.saturating_sub(1)
        };
        self.field = fields[index];
    }

    /// Left/Right change the selected row and put the new state on the air.
    pub fn adjust(&mut self, forward: bool, type_: InputType) {
        // The end of a held run: the steps have already been applied, so this
        // only has to put the result on the air.
        if type_ == InputTypeRelease {
            self.repeating = false;
            self.send();
            return;
        }

        self.step(forward);
        self.send();
    }

    /// Change the selected row by one, without transmitting.
    fn step(&mut self, forward: bool) {
        info!("A/C: adjusting a setting");

        match self.field {
            Field::OffAt => return self.step_timer(false, forward),
            Field::OnAt => return self.step_timer(true, forward),
            // Acts rather than holding a value, so either arrow does it.
            Field::TimerClear => return self.clear_timers(),
            _ => {}
        }

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
            // Lead somewhere or act; `sends` never routes these here.
            Field::Timer | Field::TimerClear => {}
            // Handled above, before the borrow.
            Field::OffAt | Field::OnAt => {}
            // The rest are flags, so either direction is just a toggle.
            Field::SwingV => ac.set_swing_vertical(!ac.swing_vertical()),
            Field::SwingH => ac.set_swing_horizontal(!ac.swing_horizontal()),
            Field::Quiet => ac.set_quiet(!ac.quiet()),
            Field::Comfort => ac.set_comfort(!ac.comfort()),
            Field::Presence => ac.set_sensor(!ac.sensor()),
            Field::Clean => ac.set_mold(!ac.mold()),
        }
    }

    fn step_timer(&mut self, on_timer: bool, forward: bool) {
        let next = daikin::next_timer(minutes_now(), self.timer_at(on_timer), forward);

        match (on_timer, next) {
            (true, Some(at)) => self.daikin.enable_on_timer(at),
            (true, None) => self.daikin.disable_on_timer(),
            (false, Some(at)) => self.daikin.enable_off_timer(at),
            (false, None) => self.daikin.disable_off_timer(),
        }
    }

    fn clear_timers(&mut self) {
        self.daikin.disable_on_timer();
        self.daikin.disable_off_timer();
    }

    /// How far off a timer is, for the caption under its row. `None` when the
    /// timer is off and there is nothing to count down to.
    pub fn timer_ahead(&self, on_timer: bool) -> Option<u16> {
        self.timer_at(on_timer)
            .map(|at| daikin::minutes_ahead(minutes_now(), at))
    }

    /// The Timer row just says whether anything is armed; the times themselves
    /// are one level down.
    pub fn timer_label(&self) -> &'static CStr {
        if self.daikin.on_timer() || self.daikin.off_timer() {
            c"On"
        } else {
            c"-"
        }
    }

    /// The clock time a timer will fire at, or `None` when it is off.
    pub fn timer_at(&self, on_timer: bool) -> Option<u16> {
        let (enabled, time) = if on_timer {
            (self.daikin.on_timer(), self.daikin.on_time())
        } else {
            (self.daikin.off_timer(), self.daikin.off_time())
        };
        enabled.then_some(time)
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
        self.daikin.set_current_time(minutes_now());
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
            Field::Timer | Field::OffAt | Field::OnAt => None,
            Field::TimerClear => None,
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
