//! Daikin ARC466A33 remote protocol.
//!
//! Port of IRremoteESP8266's `DAIKIN` protocol (the 35-byte one — `ir_Daikin.h`
//! lists ARC466A33 under it, not under DAIKIN2/312). Unlike the heater and the
//! fan, this remote is *stateful*: every button press retransmits the whole
//! 35-byte state, so there is no "press MODE once" frame to capture. That's why
//! this is built rather than recorded, and why the app's copy of the state is
//! the only thing the A/C ever hears.
//!
//! Wire format, from `IRsend::sendDaikin`:
//!
//! ```text
//! [5 zero bits, no header] gap [hdr] bytes 0..8  gap
//!                              [hdr] bytes 8..16 gap
//!                              [hdr] bytes 16..35
//! ```
//!
//! Bytes go out LSB-first. Each section carries its own checksum in its last
//! byte (a plain sum of the bytes before it).

use crate::ir::ir_press_button;
use alloc::vec::Vec;
use ufmt::derive::uDebug;

pub const STATE_LEN: usize = 35;
const SECTION1_LEN: usize = 8;
const SECTION2_LEN: usize = 8;
/// The 5 zero bits that open every message, sent bare (no header mark/space).
const LEADER_BITS: usize = 5;

pub const MIN_TEMP: u8 = 10;
pub const MAX_TEMP: u8 = 32;

/// Minutes in a day. Both timers and the frame's clock are stored as minutes
/// past midnight, so this is the modulus for all of it.
pub const DAY: u16 = 24 * 60;
/// How far one arrow press moves a timer.
pub const TIMER_STEP: u16 = 30;

/// How far ahead of `now` a timer is set for, in minutes. Wrapping the
/// subtraction is what lets a target past midnight read as "in 3h" rather than
/// "twenty hours ago".
pub fn minutes_ahead(now: u16, time: u16) -> u16 {
    (time % DAY + DAY - now % DAY) % DAY
}

/// Step a timer one notch, in the direction given.
///
/// Timers go on the wire as an absolute time of day, which is what makes a
/// retransmit idempotent — recomputing a duration on every send would restart
/// the countdown whenever an unrelated setting was touched. People think in
/// durations though, so stepping happens in "minutes from now" and converts
/// back. `current` is the time the timer is set for, or `None` when it is off;
/// the return says the same about where it should land.
pub fn next_timer(now: u16, current: Option<u16>, forward: bool) -> Option<u16> {
    let ahead = match current {
        Some(time) => minutes_ahead(now, time),
        None => 0,
    };

    let ahead = if forward {
        ahead + TIMER_STEP
    } else {
        ahead.saturating_sub(TIMER_STEP)
    };

    // Stepping down through zero switches the timer off. A full day is as far
    // ahead as the field can mean, so stepping up stops there rather than
    // wrapping round to "in a minute".
    if ahead == 0 {
        return None;
    }
    if ahead >= DAY {
        return current;
    }

    Some((now + ahead) % DAY)
}

/// Timer slots the remote parks at when the timer is off.
#[allow(dead_code)]
const UNUSED_TIME: u16 = 0x600;

mod timings {
    pub const HDR_MARK: u32 = 3650;
    pub const HDR_SPACE: u32 = 1623;
    pub const BIT_MARK: u32 = 428;
    pub const ONE_SPACE: u32 = 1280;
    pub const ZERO_SPACE: u32 = 428;
    pub const GAP: u32 = 29000;
}

use timings::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, uDebug)]
pub enum Mode {
    Auto = 0b000,
    Dry = 0b010,
    Cool = 0b011,
    Heat = 0b100,
    Fan = 0b110,
}

impl Mode {
    fn from_bits(bits: u8) -> Mode {
        match bits {
            0b010 => Mode::Dry,
            0b011 => Mode::Cool,
            0b100 => Mode::Heat,
            0b110 => Mode::Fan,
            _ => Mode::Auto,
        }
    }
}

/// The nibble the remote sends is not the speed number: auto is 0xA, quiet is
/// 0xB, and speeds 1–5 are stored as 3–7.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uDebug)]
pub enum Fan {
    Auto,
    Quiet,
    F1,
    F2,
    F3,
    F4,
    F5,
}

impl Fan {
    pub fn next(self) -> Fan {
        match self {
            Fan::Auto => Fan::Quiet,
            Fan::Quiet => Fan::F1,
            Fan::F1 => Fan::F2,
            Fan::F2 => Fan::F3,
            Fan::F3 => Fan::F4,
            Fan::F4 => Fan::F5,
            Fan::F5 => Fan::Auto,
        }
    }

    pub fn prev(self) -> Fan {
        match self {
            Fan::Auto => Fan::F5,
            Fan::Quiet => Fan::Auto,
            Fan::F1 => Fan::Quiet,
            Fan::F2 => Fan::F1,
            Fan::F3 => Fan::F2,
            Fan::F4 => Fan::F3,
            Fan::F5 => Fan::F4,
        }
    }

    fn to_bits(self) -> u8 {
        match self {
            Fan::Auto => 0b1010,
            Fan::Quiet => 0b1011,
            Fan::F1 => 3,
            Fan::F2 => 4,
            Fan::F3 => 5,
            Fan::F4 => 6,
            Fan::F5 => 7,
        }
    }

    fn from_bits(bits: u8) -> Fan {
        match bits {
            0b1011 => Fan::Quiet,
            3 => Fan::F1,
            4 => Fan::F2,
            5 => Fan::F3,
            6 => Fan::F4,
            7 => Fan::F5,
            _ => Fan::Auto,
        }
    }
}

/// The full remote state. Open-loop like every other device here: the A/C never
/// answers, so this is what we *believe* it is set to.
pub struct Daikin {
    raw: [u8; STATE_LEN],
}

impl Default for Daikin {
    fn default() -> Self {
        let mut daikin = Daikin {
            raw: RESET_STATE,
        };
        self_check();
        // The reset state is the remote's factory frame (heat, 15C, quiet fan,
        // powered on). Open on something sane instead.
        daikin.set_power(false);
        daikin.set_mode(Mode::Cool);
        daikin.set_temp(23);
        daikin.set_fan(Fan::Auto);
        daikin
    }
}

/// `IRDaikinESP::stateReset` — every byte not named here is zero, and the three
/// checksum bytes (7, 15, 34) are filled in at send time.
const RESET_STATE: [u8; STATE_LEN] = {
    let mut raw = [0u8; STATE_LEN];
    raw[0] = 0x11;
    raw[1] = 0xDA;
    raw[2] = 0x27;
    raw[4] = 0xC5;
    raw[8] = 0x11;
    raw[9] = 0xDA;
    raw[10] = 0x27;
    raw[12] = 0x42;
    raw[16] = 0x11;
    raw[17] = 0xDA;
    raw[18] = 0x27;
    raw[21] = 0x49;
    raw[22] = 0x1E;
    raw[24] = 0xB0;
    raw[27] = 0x06;
    raw[28] = 0x60;
    raw[31] = 0xC0;
    raw
};

#[allow(dead_code)] // The full remote surface, not just what this app drives.
impl Daikin {
    fn set_bit(&mut self, byte: usize, bit: u8, on: bool) {
        if on {
            self.raw[byte] |= 1 << bit;
        } else {
            self.raw[byte] &= !(1 << bit);
        }
    }

    fn bit(&self, byte: usize, bit: u8) -> bool {
        self.raw[byte] & (1 << bit) != 0
    }

    // -- Byte 21: power, timers, mode -------------------------------------

    pub fn set_power(&mut self, on: bool) {
        self.set_bit(21, 0, on);
    }

    pub fn power(&self) -> bool {
        self.bit(21, 0)
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.raw[21] = (self.raw[21] & !0b0111_0000) | ((mode as u8) << 4);
    }

    pub fn mode(&self) -> Mode {
        Mode::from_bits((self.raw[21] >> 4) & 0b111)
    }

    // -- Byte 22: temperature ---------------------------------------------

    /// Stored at half-degree resolution; this remote only does whole degrees.
    pub fn set_temp(&mut self, celsius: u8) {
        self.raw[22] = celsius.clamp(MIN_TEMP, MAX_TEMP) * 2;
    }

    pub fn temp(&self) -> u8 {
        self.raw[22] / 2
    }

    // -- Bytes 24-25: fan and swing ---------------------------------------

    pub fn set_fan(&mut self, fan: Fan) {
        self.raw[24] = (self.raw[24] & 0x0F) | (fan.to_bits() << 4);
    }

    pub fn fan(&self) -> Fan {
        Fan::from_bits(self.raw[24] >> 4)
    }

    /// The swing nibbles are all-or-nothing: 0xF on, 0x0 off.
    pub fn set_swing_vertical(&mut self, on: bool) {
        self.raw[24] = (self.raw[24] & 0xF0) | if on { 0x0F } else { 0x00 };
    }

    pub fn swing_vertical(&self) -> bool {
        self.raw[24] & 0x0F != 0
    }

    pub fn set_swing_horizontal(&mut self, on: bool) {
        self.raw[25] = (self.raw[25] & 0xF0) | if on { 0x0F } else { 0x00 };
    }

    pub fn swing_horizontal(&self) -> bool {
        self.raw[25] & 0x0F != 0
    }

    // -- Byte 29: powerful / quiet ----------------------------------------

    /// Powerful, quiet and econo are mutually exclusive on the remote.
    pub fn set_powerful(&mut self, on: bool) {
        self.set_bit(29, 0, on);
        if on {
            self.set_bit(29, 5, false);
            self.set_bit(32, 2, false);
        }
    }

    pub fn powerful(&self) -> bool {
        self.bit(29, 0)
    }

    pub fn set_quiet(&mut self, on: bool) {
        self.set_bit(29, 5, on);
        if on {
            self.set_powerful(false);
        }
    }

    pub fn quiet(&self) -> bool {
        self.bit(29, 5)
    }

    // -- Bytes 32-33: sensor, econo, mold, weekly timer --------------------

    /// "Intelligent eye" on the remote.
    pub fn set_sensor(&mut self, on: bool) {
        self.set_bit(32, 1, on);
    }

    pub fn sensor(&self) -> bool {
        self.bit(32, 1)
    }

    pub fn set_econo(&mut self, on: bool) {
        self.set_bit(32, 2, on);
        if on {
            self.set_powerful(false);
        }
    }

    pub fn econo(&self) -> bool {
        self.bit(32, 2)
    }

    /// The bit is inverted: cleared means the weekly timer is enabled.
    pub fn set_weekly_timer(&mut self, on: bool) {
        self.set_bit(32, 7, !on);
    }

    pub fn weekly_timer(&self) -> bool {
        !self.bit(32, 7)
    }

    /// Mould-proof / dry-out fan run after shutdown.
    pub fn set_mold(&mut self, on: bool) {
        self.set_bit(33, 1, on);
    }

    pub fn mold(&self) -> bool {
        self.bit(33, 1)
    }

    // -- Byte 6: comfort ---------------------------------------------------

    pub fn set_comfort(&mut self, on: bool) {
        self.set_bit(6, 4, on);
    }

    pub fn comfort(&self) -> bool {
        self.bit(6, 4)
    }

    // -- Bytes 13-14: the remote's own clock -------------------------------

    /// Minutes past midnight. The real remote stamps every frame with its
    /// clock; the A/C needs it for the weekly and on/off timers.
    pub fn set_current_time(&mut self, mins_past_midnight: u16) {
        let mins = if mins_past_midnight > 24 * 60 {
            0
        } else {
            mins_past_midnight
        };
        let day = (self.raw[14] >> 3) & 0b111;
        self.raw[13] = mins as u8;
        self.raw[14] = ((mins >> 8) as u8 & 0b111) | (day << 3);
    }

    /// SUN=1, MON=2, ... SAT=7.
    pub fn set_current_day(&mut self, day: u8) {
        self.raw[14] = (self.raw[14] & 0b0000_0111) | ((day & 0b111) << 3);
    }

    // -- Bytes 26-28: on/off timers ---------------------------------------

    pub fn on_timer(&self) -> bool {
        self.bit(21, 1)
    }

    pub fn off_timer(&self) -> bool {
        self.bit(21, 2)
    }

    /// Minutes past midnight. Meaningless unless the matching enable bit is
    /// set — a disabled timer parks at `UNUSED_TIME`.
    pub fn on_time(&self) -> u16 {
        u16::from_le_bytes([self.raw[26], self.raw[27]]) & 0x0FFF
    }

    pub fn off_time(&self) -> u16 {
        (self.raw[27] >> 4) as u16 | ((self.raw[28] as u16) << 4)
    }

    pub fn enable_on_timer(&mut self, mins_past_midnight: u16) {
        self.set_bit(21, 1, true);
        self.set_on_time(mins_past_midnight);
    }

    pub fn disable_on_timer(&mut self) {
        self.set_bit(21, 1, false);
        self.set_on_time(UNUSED_TIME);
    }

    pub fn enable_off_timer(&mut self, mins_past_midnight: u16) {
        self.set_bit(21, 2, true);
        self.set_off_time(mins_past_midnight);
    }

    pub fn disable_off_timer(&mut self) {
        self.set_bit(21, 2, false);
        self.set_off_time(UNUSED_TIME);
    }

    /// Both times share bytes 26–28 as one 24-bit little-endian field: on time
    /// in the low 12 bits, off time in the high 12.
    fn set_on_time(&mut self, mins: u16) {
        let mins = mins & 0x0FFF;
        self.raw[26] = mins as u8;
        self.raw[27] = (self.raw[27] & 0xF0) | ((mins >> 8) as u8 & 0x0F);
    }

    fn set_off_time(&mut self, mins: u16) {
        let mins = mins & 0x0FFF;
        self.raw[27] = (self.raw[27] & 0x0F) | ((mins as u8 & 0x0F) << 4);
        self.raw[28] = (mins >> 4) as u8;
    }

    // -- Raw access --------------------------------------------------------

    /// The state as it sits on the wire, minus the checksums — those are
    /// recomputed on every send, so they are not stored.
    pub fn raw(&self) -> &[u8; STATE_LEN] {
        &self.raw
    }

    /// Adopt a state from outside. Rejects anything that doesn't carry the
    /// three fixed section headers, which is enough to catch a truncated or
    /// corrupt file without pretending to validate the rest.
    pub fn from_raw(raw: [u8; STATE_LEN]) -> Option<Daikin> {
        for section in [0, SECTION1_LEN, SECTION1_LEN + SECTION2_LEN] {
            if raw[section..section + 3] != [0x11, 0xDA, 0x27] {
                return None;
            }
        }
        Some(Daikin { raw })
    }

    // -- Sending -----------------------------------------------------------

    /// Fill in the three section checksums and put the whole state on the air.
    pub fn send(&self) {
        let mut raw = self.raw;
        checksum(&mut raw);
        ir_press_button(&encode(&raw));
    }
}

/// Each section's last byte is the sum of the bytes before it, mod 256.
fn checksum(raw: &mut [u8; STATE_LEN]) {
    raw[SECTION1_LEN - 1] = sum(&raw[..SECTION1_LEN - 1]);
    raw[SECTION1_LEN + SECTION2_LEN - 1] =
        sum(&raw[SECTION1_LEN..SECTION1_LEN + SECTION2_LEN - 1]);
    raw[STATE_LEN - 1] = sum(&raw[SECTION1_LEN + SECTION2_LEN..STATE_LEN - 1]);
}

fn sum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, b| acc.wrapping_add(*b))
}

fn encode(raw: &[u8; STATE_LEN]) -> Vec<u32> {
    let mut out = Vec::with_capacity(TIMINGS_LEN);

    // The leader: five zero bits with no header of their own.
    for _ in 0..LEADER_BITS {
        out.push(BIT_MARK);
        out.push(ZERO_SPACE);
    }
    push_footer(&mut out);

    push_section(&mut out, &raw[..SECTION1_LEN]);
    push_section(&mut out, &raw[SECTION1_LEN..SECTION1_LEN + SECTION2_LEN]);
    push_section(&mut out, &raw[SECTION1_LEN + SECTION2_LEN..]);

    // Drop the trailing inter-section gap: nothing follows it, and the send
    // must end on a mark.
    out.pop();
    debug_assert_eq!(out.len(), TIMINGS_LEN);
    out
}

/// Leader (5 bits + footer) + three header/footer-wrapped sections.
const TIMINGS_LEN: usize = LEADER_BITS * 2 + 2 + (2 + 2) * 3 + STATE_LEN * 8 * 2 - 1;

fn push_section(out: &mut Vec<u32>, bytes: &[u8]) {
    out.push(HDR_MARK);
    out.push(HDR_SPACE);
    for byte in bytes {
        // LSB first
        for bit in 0..8 {
            out.push(BIT_MARK);
            out.push(if byte >> bit & 1 == 1 {
                ONE_SPACE
            } else {
                ZERO_SPACE
            });
        }
    }
    push_footer(out);
}

fn push_footer(out: &mut Vec<u32>) {
    out.push(BIT_MARK);
    out.push(ZERO_SPACE + GAP);
}

/// Pins the byte layout and the checksums against a real ARC-series capture
/// from IRremoteESP8266's test suite (`TestDecodeDaikin.RealExample`), which
/// decodes to: on, cool, 29C, auto fan, powerful, clock 22:18, on timer 21:30,
/// off timer 06:10.
///
/// Runs once, from `default()` — there is no test harness on the Flipper, and a
/// wrong frame here is a silent no-op on the A/C rather than an obvious crash.
fn self_check() {
    const CAPTURED: [u8; STATE_LEN] = [
        0x11, 0xDA, 0x27, 0x00, 0xC5, 0x00, 0x00, 0xD7, 0x11, 0xDA, 0x27, 0x00, 0x42, 0x3A, 0x05,
        0x93, 0x11, 0xDA, 0x27, 0x00, 0x00, 0x3F, 0x3A, 0x00, 0xA0, 0x00, 0x0A, 0x25, 0x17, 0x01,
        0x00, 0xC0, 0x00, 0x00, 0x32,
    ];

    // Every setter used below has to land on exactly the captured bytes.
    let mut built = Daikin { raw: RESET_STATE };
    built.set_power(true);
    built.set_mode(Mode::Cool);
    built.set_temp(29);
    built.set_fan(Fan::Auto);
    built.set_swing_vertical(false);
    built.set_swing_horizontal(false);
    built.set_powerful(true);
    built.set_current_time(22 * 60 + 18);
    built.set_current_day(0);
    built.enable_on_timer(21 * 60 + 30);
    built.enable_off_timer(6 * 60 + 10);
    checksum(&mut built.raw);
    assert!(built.raw == CAPTURED, "Daikin state does not match the capture");

    // ...and every getter has to read them back.
    let captured = Daikin { raw: CAPTURED };
    assert!(captured.power(), "Daikin power");
    assert!(captured.mode() == Mode::Cool, "Daikin mode");
    assert_eq!(captured.temp(), 29, "Daikin temp");
    assert!(captured.fan() == Fan::Auto, "Daikin fan");
    assert!(captured.powerful(), "Daikin powerful");
    assert!(!captured.quiet(), "Daikin quiet");
    assert!(captured.weekly_timer(), "Daikin weekly timer");
    assert!(captured.on_timer() && captured.off_timer(), "Daikin timer flags");
    assert_eq!(captured.on_time(), 21 * 60 + 30, "Daikin on time");
    assert_eq!(captured.off_time(), 6 * 60 + 10, "Daikin off time");

    assert_eq!(encode(&CAPTURED).len(), TIMINGS_LEN, "Daikin frame length");
}
