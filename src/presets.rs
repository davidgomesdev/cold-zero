//! Named snapshots of the whole remote state.
//!
//! One preset is always the live one: [`crate::ac::AcState`] edits its `daikin`
//! and writes it back here on every save, so the list on disk is what the app
//! believes about the A/C, not a separate copy that could drift from it.

use crate::daikin::{Daikin, Fan, Mode, STATE_LEN};
use alloc::vec::Vec;
use core::ffi::{CStr, c_char};
use flipperzero::io::{Read, Write};
use flipperzero::storage::{File, Storage};
use flipperzero::{info, warn};
use flipperzero_sys::storage_common_mkdir;

/// Room for the name plus its terminator. Kept short because the row that
/// shows it is 63px wide and shares them with the "Preset" label.
pub const NAME_LEN: usize = 9;

/// The two that ship with the app. They can be edited like any other, and the
/// last row puts them back.
const BUILTIN: [(&[u8], u8, Mode, Fan); 2] = [
    (b"Day", 23, Mode::Cool, Fan::Auto),
    (b"Sleep", 25, Mode::Cool, Fan::Quiet),
];

/// A cap so the file stays a known size and the ring stays walkable by hand.
const MAX: usize = 8;

const DIR: &CStr = c"/ext/apps_data/cold-zero";
const PATH: &CStr = c"/ext/apps_data/cold-zero/presets.bin";
/// What the app saved before presets existed: a single bare state. Read once,
/// so an upgrade doesn't throw away what the A/C was last told.
const LEGACY_PATH: &CStr = c"/ext/apps_data/cold-zero/ac.bin";

const ENTRY_LEN: usize = NAME_LEN + STATE_LEN;

pub struct Preset {
    /// NUL-terminated ASCII, so the draw code can hand it straight to the
    /// canvas without building a `CString` every frame.
    name: [u8; NAME_LEN],
    raw: [u8; STATE_LEN],
}

impl Preset {
    fn new(name: &[u8], raw: [u8; STATE_LEN]) -> Preset {
        let mut preset = Preset {
            name: [0; NAME_LEN],
            raw,
        };
        let len = name.len().min(NAME_LEN - 1);
        preset.name[..len].copy_from_slice(&name[..len]);
        preset
    }

    pub fn label(&self) -> *const c_char {
        self.name.as_ptr().cast()
    }

    pub fn raw(&self) -> &[u8; STATE_LEN] {
        &self.raw
    }
}

pub struct Presets {
    list: Vec<Preset>,
    /// The preset the live state belongs to.
    selected: usize,
    /// What the Preset row is showing. Equal to `selected` except while the
    /// cursor is parked on the "Create..." slot past the end of the ring,
    /// which is a place to press OK rather than a preset to load.
    slot: usize,
}

impl Presets {
    /// The built-ins as they ship.
    fn factory(index: usize) -> Preset {
        let (name, temp, mode, fan) = BUILTIN[index];
        let mut ac = Daikin::default();
        ac.set_power(true);
        ac.set_mode(mode);
        ac.set_temp(temp);
        ac.set_fan(fan);
        if index == 1 {
            // Sleep: as quiet and as cheap as the unit will run.
            ac.set_quiet(true);
            ac.set_econo(true);
        }
        Preset::new(name, *ac.raw())
    }

    fn default_list() -> Presets {
        Presets {
            list: (0..BUILTIN.len()).map(Presets::factory).collect(),
            selected: 0,
            slot: 0,
        }
    }

    pub fn load() -> Presets {
        let mut buffer = [0u8; 2 + MAX * ENTRY_LEN];
        let read = match File::open(PATH).and_then(|mut file| file.read(&mut buffer)) {
            Ok(read) => read,
            Err(_) => {
                info!("A/C: no presets yet, starting from the built-ins");
                return Presets::migrate();
            }
        };

        match Presets::parse(&buffer[..read]) {
            Some(presets) => {
                info!("A/C: restored the presets");
                presets
            }
            None => {
                warn!("A/C: presets file is corrupt, starting from the built-ins");
                Presets::default_list()
            }
        }
    }

    fn parse(bytes: &[u8]) -> Option<Presets> {
        let count = *bytes.first()? as usize;
        let selected = *bytes.get(1)? as usize;
        if !(BUILTIN.len()..=MAX).contains(&count) || selected >= count {
            return None;
        }
        let body = bytes.get(2..2 + count * ENTRY_LEN)?;

        let mut list = Vec::with_capacity(count);
        for entry in body.chunks_exact(ENTRY_LEN) {
            let mut name = [0u8; NAME_LEN];
            name.copy_from_slice(&entry[..NAME_LEN]);
            // A name that lost its terminator would run off the end of the
            // array the moment the canvas read it.
            if name[NAME_LEN - 1] != 0 {
                return None;
            }
            let mut raw = [0u8; STATE_LEN];
            raw.copy_from_slice(&entry[NAME_LEN..]);
            // The same header check the single-state file always had.
            Daikin::from_raw(raw)?;
            list.push(Preset { name, raw });
        }

        Some(Presets {
            list,
            selected,
            slot: selected,
        })
    }

    /// First launch after the upgrade: seed Day with whatever the app last
    /// told the A/C, rather than a guess it never sent.
    fn migrate() -> Presets {
        let mut presets = Presets::default_list();

        let mut raw = [0u8; STATE_LEN];
        if let Ok(STATE_LEN) = File::open(LEGACY_PATH).and_then(|mut file| file.read(&mut raw))
            && Daikin::from_raw(raw).is_some()
        {
            info!("A/C: carried the saved state into the Day preset");
            presets.list[0].raw = raw;
        }

        presets
    }

    /// Best-effort: a Flipper with no SD card still has to work.
    pub fn save(&self) {
        unsafe { storage_common_mkdir(Storage::open().as_ptr(), DIR.as_ptr()) };

        let bytes = self.encode();
        let saved = File::create(PATH).and_then(|mut file| {
            file.write_all(&bytes)?;
            file.flush()
        });

        if saved.is_err() {
            warn!("A/C: could not save the presets");
        }
    }

    /// The file's other half, kept next to [`Presets::parse`] so the two stay
    /// the same shape.
    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(2 + self.list.len() * ENTRY_LEN);
        bytes.push(self.list.len() as u8);
        bytes.push(self.selected as u8);
        for preset in &self.list {
            bytes.extend_from_slice(&preset.name);
            bytes.extend_from_slice(&preset.raw);
        }
        bytes
    }

    pub fn current(&self) -> &Preset {
        &self.list[self.selected]
    }

    /// What the Preset row shows: a name, or the slot past the end.
    pub fn label(&self) -> *const c_char {
        match self.list.get(self.slot) {
            Some(preset) => preset.label(),
            None => c"Create...".as_ptr(),
        }
    }

    /// Whether the row is parked on the slot that makes a new preset.
    pub fn creating(&self) -> bool {
        self.slot >= self.list.len()
    }

    /// Built-ins get put back; the rest get removed.
    pub fn builtin(&self) -> bool {
        self.selected < BUILTIN.len()
    }

    /// Whether stepping would land on a preset — a "Create..." landing changes
    /// nothing and so must not transmit.
    pub fn lands(&self, forward: bool) -> bool {
        self.stepped(forward) < self.list.len()
    }

    fn stepped(&self, forward: bool) -> usize {
        // The ring is the presets plus the one slot past them, except once the
        // list is full: then there is nothing to create.
        let ring = if self.list.len() < MAX {
            self.list.len() + 1
        } else {
            self.list.len()
        };
        if forward {
            (self.slot + 1) % ring
        } else {
            (self.slot + ring - 1) % ring
        }
    }

    /// Move the row. Returns the state to load, or `None` when the new slot is
    /// "Create...", which leaves the live state alone.
    pub fn step(&mut self, forward: bool) -> Option<[u8; STATE_LEN]> {
        self.slot = self.stepped(forward);
        if self.creating() {
            return None;
        }
        self.selected = self.slot;
        Some(self.list[self.selected].raw)
    }

    /// Leaving the row: nothing edits a slot that holds no preset, so the row
    /// goes back to showing the one the live state actually belongs to.
    pub fn leave(&mut self) {
        self.slot = self.selected;
    }

    /// Write the live state back into the preset it came from. Every save goes
    /// through here, which is what keeps the file and the belief the same.
    pub fn store(&mut self, raw: [u8; STATE_LEN]) {
        if let Some(preset) = self.list.get_mut(self.selected) {
            preset.raw = raw;
        }
    }

    /// A new preset holds what is on screen now — the reason to name one is
    /// usually that the current settings are worth keeping.
    pub fn create(&mut self, name: &[u8], raw: [u8; STATE_LEN]) {
        if self.list.len() >= MAX {
            return;
        }
        self.list.push(Preset::new(name, raw));
        self.selected = self.list.len() - 1;
        self.slot = self.selected;
        self.save();
    }

    /// The last row: put a built-in back the way it shipped, or drop a custom
    /// one and fall back to its neighbour. Returns the state to load, since
    /// either way the live one is now somebody else's.
    pub fn reset_or_delete(&mut self) -> [u8; STATE_LEN] {
        if self.builtin() {
            self.list[self.selected] = Presets::factory(self.selected);
        } else {
            self.list.remove(self.selected);
            self.selected = self.selected.min(self.list.len() - 1);
        }
        self.slot = self.selected;
        self.save();
        self.list[self.selected].raw
    }
}
