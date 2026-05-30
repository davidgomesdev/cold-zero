# Multi-Device Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Tower Fan as a second controllable device alongside the existing Heater, with Left/Right switching on a single screen.

**Architecture:** `FanState` lives in a new `src/fan.rs` module (parallel to `HeaterState` in `state.rs`). `AppState` gains `fan_state: FanState` and `active_device: ActiveDevice` alongside the existing `heater_state`. The draw callback and input handler both match on `active_device` to dispatch device-specific behaviour. The daytime auto-trigger fires only when `active_device == ActiveDevice::Heater`. No dynamic dispatch — `ActiveDevice` is a plain enum discriminant.

**Tech Stack:** no_std Rust (nightly-2025-08-31), `flipperzero` / `flipperzero-sys` crates, `ufmt` for formatting. Verify with `cargo check`; final verify with `cargo build --release`.

---

## File Map

| File | Action | Responsibility |
|---|---|---|
| `src/ir.rs` | Modify | Add `pub mod fan` submodule with mock IR timings |
| `src/fan.rs` | Create | `FanState`, `FanLight`, `FanMode`, `power_on`, `power_off` |
| `src/state.rs` | Modify | Add `ActiveDevice` enum; add `fan_state` + `active_device` to `AppState` |
| `src/main.rs` | Modify | Module declaration, AppState init, input dispatch, daytime guard, draw |

---

## Task 1: Fan IR mock timings

**Files:**
- Modify: `src/ir.rs`

- [ ] **Step 1: Add `fan` submodule to `src/ir.rs`**

Append at the very end of `src/ir.rs`, after the closing `}` of the existing `timings` module:

```rust
pub mod fan {
    pub const FREQUENCY: u32 = 38000;
    pub const DUTY_CYCLE: f32 = 0.33;

    /// Mock — replace with pulse array captured from the real remote
    pub const POWER_BTN: [u32; 1] = [0];
    /// Mock — replace with pulse array captured from the real remote
    pub const TIMER_BTN: [u32; 1] = [0];
    /// Mock — replace with pulse array captured from the real remote
    pub const LIGHT_BTN: [u32; 1] = [0];
}
```

- [ ] **Step 2: Verify**

```bash
cargo check
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src/ir.rs
git commit -m "feat: add mock fan IR timings"
```

---

## Task 2: FanState module

**Files:**
- Create: `src/fan.rs`
- Modify: `src/main.rs` (add `mod fan;` only)

- [ ] **Step 1: Create `src/fan.rs`**

```rust
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
}

#[derive(Debug, PartialEq, Eq, uDebug)]
pub enum FanLight {
    Full,
    Partial,
    Off,
}

#[derive(Debug, PartialEq, Eq, uDebug)]
pub enum FanMode {
    F1,
    F2,
    F3,
    Sleep,
    Nature,
}

impl Default for FanState {
    fn default() -> Self {
        FanState {
            is_on: false,
            timer: 0,
            light: FanLight::Full,
            fan_mode: FanMode::F2,
        }
    }
}

impl FanState {
    pub fn power_on(&mut self) {
        info!("Fan: powering on");
        ir_press_button(&fan_ir::POWER_BTN);
        // First TIMER press is ignored (same physical quirk as heater's warmer button)
        ir_press_button(&fan_ir::TIMER_BTN);
        ir_press_button(&fan_ir::TIMER_BTN);
        // LIGHT must come last — any other button press reactivates the light
        ir_press_button(&fan_ir::LIGHT_BTN); // Full -> Partial
        ir_press_button(&fan_ir::LIGHT_BTN); // Partial -> Off

        self.is_on = true;
        self.timer = 1;
        self.light = FanLight::Off;
        // fan_mode stays F2 (fan defaults to F2 on power on)
    }

    pub fn power_off(&mut self) {
        info!("Fan: powering off");
        ir_press_button(&fan_ir::POWER_BTN);

        self.is_on = false;
        self.timer = 0;
        self.light = FanLight::Full;
    }
}
```

- [ ] **Step 2: Register module in `src/main.rs`**

In `src/main.rs`, replace the module declarations block (lines 8–11):

```rust
mod allocator;
mod fan;
mod ir;
mod notification;
mod state;
```

- [ ] **Step 3: Verify**

```bash
cargo check
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src/fan.rs src/main.rs
git commit -m "feat: add FanState module"
```

---

## Task 3: Add ActiveDevice to AppState

**Files:**
- Modify: `src/state.rs`
- Modify: `src/main.rs` (imports + AppState initialiser — fixes compile break from state change)

- [ ] **Step 1: Add `FanState` import and `ActiveDevice` enum to `src/state.rs`**

Add at the top of `src/state.rs`, after the existing `use` statements:

```rust
use crate::fan::FanState;
```

Then add the `ActiveDevice` enum after the existing `RunState` enum:

```rust
#[derive(PartialEq, Eq)]
pub enum ActiveDevice {
    Heater,
    Fan,
}
```

- [ ] **Step 2: Add new fields to `AppState` in `src/state.rs`**

Replace the existing `AppState` struct:

```rust
pub struct AppState {
    pub last_daytime_run_day: u8,
    pub heater_state: HeaterState,
    pub fan_state: FanState,
    pub active_device: ActiveDevice,
    pub run_state: RunState,
    pub mutex: *mut FuriMutex,
}
```

- [ ] **Step 3: Update imports in `src/main.rs`**

Replace line 14:
```rust
use crate::state::{HeaterMode, HeaterState, RunState};
```
With:
```rust
use crate::fan::FanState;
use crate::state::{ActiveDevice, HeaterMode, HeaterState, RunState};
```

- [ ] **Step 4: Update `AppState` initialisation in `src/main.rs`**

In `run()`, replace the `Box::new(AppState { ... })` call:

```rust
let app_state = Box::into_raw(Box::new(AppState {
    heater_state: HeaterState::default(),
    fan_state: FanState::default(),
    active_device: ActiveDevice::Heater,
    run_state: RunState::WaitingForDaytime,
    last_daytime_run_day: 0,
    mutex: furi_mutex_alloc(FuriMutexTypeNormal),
}));
```

- [ ] **Step 5: Verify**

```bash
cargo check
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add src/state.rs src/main.rs
git commit -m "feat: add ActiveDevice enum and fan_state to AppState"
```

---

## Task 4: Input handling — device cycling and OK dispatch

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Add `InputKeyLeft` and `InputKeyRight` to `flipperzero_sys` imports**

Replace lines 30–31 in the `use flipperzero_sys::{...}` block:

```rust
    Gui, GuiLayerFullscreen, InputEvent, InputKeyBack, InputKeyLeft, InputKeyRight,
    InputKeyOk, InputTypeLong, InputTypeShort, ViewPort,
```

- [ ] **Step 2: Add `cycle_device` function**

Add after `start_of_day_power_heater`:

```rust
fn cycle_device(app_state: &mut AppState) {
    app_state.active_device = match app_state.active_device {
        ActiveDevice::Heater => ActiveDevice::Fan,
        ActiveDevice::Fan => ActiveDevice::Heater,
    };
}
```

- [ ] **Step 3: Update `handle_key_presses` match to handle Left/Right**

Replace the `match input_event.key` block inside `handle_key_presses`:

```rust
match input_event.key {
    InputKeyBack => {
        return false;
    }
    InputKeyOk => handle_ok_press(notification_app, app_state, input_event),
    InputKeyLeft | InputKeyRight => cycle_device(app_state),
    key => {
        debug!("Received input that is not handled ({})", key.0);
    }
}
```

- [ ] **Step 4: Refactor `handle_ok_press` into a dispatcher + two device handlers**

Replace the entire `handle_ok_press` function with these three functions:

```rust
#[allow(non_upper_case_globals)]
fn handle_ok_press(
    notification_app: &mut NotificationApp,
    app_state: &mut AppState,
    input_event: InputEvent,
) {
    match app_state.active_device {
        ActiveDevice::Heater => handle_heater_ok_press(notification_app, app_state, input_event),
        ActiveDevice::Fan => handle_fan_ok_press(app_state, input_event),
    }
    app_state.run_state = RunState::WaitingForDaytime;
}

#[allow(non_upper_case_globals)]
fn handle_heater_ok_press(
    notification_app: &mut NotificationApp,
    app_state: &mut AppState,
    input_event: InputEvent,
) {
    if (input_event.type_ == InputTypeLong || input_event.type_ == InputTypeShort)
        && app_state.heater_state.is_on
    {
        app_state.heater_state.power_off();
        notification_app.notify(&MANUAL_POWER_OFF);
        return;
    }

    match input_event.type_ {
        InputTypeShort => {
            app_state.heater_state.power_on();
            notification_app.notify(&MANUAL_POWER_ON);
        }
        InputTypeLong => {
            start_of_day_power_heater(notification_app, app_state);
        }
        _ => {
            debug!(
                "Received OK button press type not handled ({})",
                input_event.type_.0
            );
        }
    }
}

#[allow(non_upper_case_globals)]
fn handle_fan_ok_press(app_state: &mut AppState, input_event: InputEvent) {
    if input_event.type_ != InputTypeShort && input_event.type_ != InputTypeLong {
        return;
    }
    if app_state.fan_state.is_on {
        app_state.fan_state.power_off();
    } else {
        app_state.fan_state.power_on();
    }
}
```

- [ ] **Step 5: Guard daytime auto-trigger for Heater only**

In `run()`, find:

```rust
if app_state.last_daytime_run_day < time.day {
```

Replace with:

```rust
if app_state.last_daytime_run_day < time.day
    && app_state.active_device == ActiveDevice::Heater
{
```

- [ ] **Step 6: Verify**

```bash
cargo check
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add src/main.rs
git commit -m "feat: Left/Right device cycling and per-device OK dispatch"
```

---

## Task 5: Device-specific draw callbacks

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Replace `on_draw` and add two draw helper functions**

Replace the entire `on_draw` function with:

```rust
unsafe extern "C" fn on_draw(canvas: *mut Canvas, app_state: *mut c_void) {
    unsafe {
        let app_state: &AppState = &*(app_state as *const AppState);
        match app_state.active_device {
            ActiveDevice::Heater => draw_heater(canvas, app_state),
            ActiveDevice::Fan => draw_fan(canvas, app_state),
        }
    }
}

unsafe fn draw_heater(canvas: *mut Canvas, app_state: &AppState) {
    unsafe {
        canvas_draw_str(canvas, 0, 10, c"< Heater >".as_ptr());

        let status = match app_state.run_state {
            RunState::WaitingForDaytime => c"Waiting for daytime...".as_ptr(),
            RunState::Changing => c"Changing heater state...".as_ptr(),
            RunState::SetDaytimeHeat => c"Heater set for daytime!".as_ptr(),
        };
        canvas_draw_str(canvas, 0, 20, status);

        let mode_str = match app_state.heater_state.mode {
            HeaterMode::HeatLow => "HeatLow",
            HeaterMode::HeatHigh => "HeatHigh",
            HeaterMode::Eco => "Eco",
        };
        let heater_str = format!(
            "Heater: {} {}C {}",
            if app_state.heater_state.is_on { "ON" } else { "OFF" },
            app_state.heater_state.temperature,
            mode_str,
        );
        canvas_draw_str(canvas, 0, 30, heater_str.as_ptr());

        let time_str = format!(
            "{}:{}:{}",
            datetime().hour,
            datetime().minute,
            datetime().second,
        );
        canvas_draw_str(canvas, 0, 48, time_str.as_ptr());

        let hints = if app_state.heater_state.is_on {
            c"OK:off Hold:off <>:sw".as_ptr()
        } else {
            c"OK:on Hold:day <>:sw".as_ptr()
        };
        canvas_draw_str(canvas, 0, 60, hints);
    }
}

unsafe fn draw_fan(canvas: *mut Canvas, app_state: &AppState) {
    use crate::fan::FanLight;
    unsafe {
        canvas_draw_str(canvas, 0, 10, c"< Fan >".as_ptr());

        let on_str = if app_state.fan_state.is_on {
            c"Fan: ON".as_ptr()
        } else {
            c"Fan: OFF".as_ptr()
        };
        canvas_draw_str(canvas, 0, 20, on_str);

        if app_state.fan_state.is_on {
            let light_str = match app_state.fan_state.light {
                FanLight::Full => "Full",
                FanLight::Partial => "Part",
                FanLight::Off => "Off",
            };
            let fan_detail = format!(
                "Light:{} Timer:{}h F2",
                light_str,
                app_state.fan_state.timer,
            );
            canvas_draw_str(canvas, 0, 30, fan_detail.as_ptr());
        } else {
            canvas_draw_str(canvas, 0, 30, c"Light:- Timer:- F2".as_ptr());
        }

        let time_str = format!(
            "{}:{}:{}",
            datetime().hour,
            datetime().minute,
            datetime().second,
        );
        canvas_draw_str(canvas, 0, 48, time_str.as_ptr());

        let hints = if app_state.fan_state.is_on {
            c"OK:off  <>:switch".as_ptr()
        } else {
            c"OK:on  <>:switch".as_ptr()
        };
        canvas_draw_str(canvas, 0, 60, hints);
    }
}
```

- [ ] **Step 2: Build**

```bash
cargo build --release
```

Expected: build succeeds, produces `target/thumbv7em-none-eabihf/release/cold-zero.fap`.

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat: device-specific draw callbacks with hints bar"
```
