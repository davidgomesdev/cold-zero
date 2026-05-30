# Multi-Device Support Design

**Date:** 2026-05-30
**Status:** Approved

## Overview

Add a Tower Fan as a second controllable device alongside the existing Heater. The user switches between devices horizontally (Left/Right) on a single screen. Each device has its own state, IR sequence, and display. The fan is manual-only (no daytime auto-trigger).

---

## State & Data Model

### `src/fan.rs` (new)

```rust
pub struct FanState {
    pub is_on: bool,
    pub timer: u8,         // 0–9 hours; starts at 0
    pub light: FanLight,   // starts at Full
    pub fan_mode: FanMode, // always F2 for now
}

pub enum FanLight { Full, Partial, Off }
pub enum FanMode  { F1, F2, F3, Sleep, Nature }
```

Defaults: `is_on: false`, `timer: 0`, `light: Full`, `fan_mode: F2`.

### `src/state.rs` (modified)

Add `ActiveDevice` enum, replace `heater_state: HeaterState` in `AppState`:

```rust
pub enum ActiveDevice {
    Heater(HeaterState),
    Fan(FanState),
}

pub struct AppState {
    pub active_device: ActiveDevice,  // replaces heater_state
    pub run_state: RunState,
    pub last_daytime_run_day: u8,
    pub mutex: *mut FuriMutex,
}
```

Default: `active_device: ActiveDevice::Heater(HeaterState::default())`.

---

## Fan Power Sequence

### `power_on()`
IR commands sent in this exact order (LIGHT must be last — any other button press reactivates the light):

1. `POWER` × 1
2. `TIMER` × 2 — cycles 0 → 1 (first press ignored, same quirk as heater's warmer button)
3. `LIGHT` × 2 — cycles Full → Partial → Off

State after: `is_on: true`, `timer: 1`, `light: Off`, `fan_mode: F2` (unchanged).

### `power_off()`
1. `POWER` × 1

State after: all fields reset to defaults.

---

## Input Handling

| Input | Heater active | Fan active |
|---|---|---|
| **Left / Right** | Switch to Fan | Switch to Heater |
| **OK short** (device off) | Power on (Eco, 23°C) | Power on (full sequence above) |
| **OK short** (device on) | Power off | Power off |
| **OK long** (device off) | Daytime sequence (HeatHigh, 35°C) | Power on (same as short) |
| **OK long** (device on) | Power off | Power off |
| **Back** | Exit app | Exit app |

Switching devices keeps each device's in-memory state intact for the session.

The daytime auto-trigger in the main loop (`start_of_day_power_heater`) only fires when `active_device` is `ActiveDevice::Heater(_)`.

---

## Display

Five lines on the 128×64 monochrome screen. The device indicator (`< Heater >` / `< Fan >`) is on line 1.

### Heater (off)
```
< Heater >
Waiting for daytime...
Heater: OFF 23°C Eco
08:00:00
OK:on  Hold:daytime  ◄►:switch
```

### Heater (on)
```
< Heater >
SetDaytimeHeat
Heater: ON 35°C HeatHigh
08:00:00
OK:off  Hold:off  ◄►:switch
```

### Fan (off)
```
< Fan >
Fan: OFF
Light:-  Timer:-  F2
08:00:00
OK:on  ◄►:switch
```

### Fan (on)
```
< Fan >
Fan: ON
Light:Off  Timer:1h  F2
08:00:00
OK:off  ◄►:switch
```

---

## IR Module

New `fan` submodule in `src/ir.rs` (mirrors existing `timings` submodule):

```rust
pub mod fan {
    pub const FREQUENCY: u32 = 38000;  // placeholder — verify against real remote
    pub const DUTY_CYCLE: f32 = 0.33;  // placeholder — verify against real remote

    pub const POWER_BTN: [u32; 1] = [0];  // mock — replace with real capture
    pub const TIMER_BTN: [u32; 1] = [0];  // mock — replace with real capture
    pub const LIGHT_BTN: [u32; 1] = [0];  // mock — replace with real capture
}
```

`ir_press_button(&[u32])` is reused as-is for fan buttons.

---

## Files Changed

| File | Change |
|---|---|
| `src/fan.rs` | New — `FanState`, `FanLight`, `FanMode`, `power_on`, `power_off` |
| `src/state.rs` | Add `ActiveDevice` enum; replace `heater_state` field in `AppState` |
| `src/ir.rs` | Add `fan` submodule with mock timings |
| `src/main.rs` | Left/Right cycling, OK/long-OK dispatch per device, draw per device, hints bar |
