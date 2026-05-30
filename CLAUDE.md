# CLAUDE.md

## What this is

Flipper Zero app (`.fap`) written in `no_std` Rust. Controls a physical heater via IR blaster. Automatically powers/configures the heater at daytime hours; manual override via OK button.

Target: `thumbv7em-none-eabihf` (Flipper Zero's STM32WB55 MCU). Nightly Rust required (`rust-toolchain.toml` pins `nightly-2025-08-31`).

## Commands

```sh
# First-time setup (installs flipperzero-tools CLI + creates storage dir on device)
make setup

# Build
cargo build --release

# Build + deploy to connected Flipper Zero
make install
```

No tests — `test = false` in Cargo.toml. No linting config beyond `cargo check`.

## Architecture

Single binary, event loop in `run()` (`main.rs`). Key modules:

- **`state`** — `AppState` (shared across draw/input callbacks via mutex), `HeaterState` (tracks on/off, temp 5–35°C, mode), `RunState` (state machine: `WaitingForDaytime` → `Changing` → `SetDaytimeHeat`)
- **`ir`** — sends raw IR signals via `infrared_send_raw_ext`. All button timings (POWER, MODE, WARMER, COOLER) are hardcoded pulse arrays captured from the real remote
- **`notification`** — LED feedback sequences for power on/off and daytime events
- **`allocator`** — thin wrapper around Flipper's `malloc`/`free` as the global allocator (required for `no_std` + `alloc`)

### Control flow

`run()` loop acquires mutex each tick, checks RTC time, fires `start_of_day_power_heater()` once per day (weekdays 08:00–13:00, weekends 09:00–13:00 via `last_daytime_run_day`), then drains the input queue with 100ms timeout.

OK short press → power on (eco, 23°C). OK long press → daytime sequence (HeatHigh, 35°C). Either press when on → power off. Back → exit app.

### IR quirk

`set_temp` sends one extra button press (`change_needed.abs() + 1`) because the first warmer/cooler press doesn't register on the physical remote.

### Furi concurrency model

Draw callback (`on_draw`) and input callback (`on_input`) run on different Furi threads. Input callback only enqueues events; all state mutation happens in the main loop under the mutex.

## Key conventions

- **IR timings are raw pulse arrays** — values are microsecond on/off durations captured from the physical remote. Never synthesise them; always capture from the real device.
- **`HeaterMode::next()` cycles** `HeatLow → HeatHigh → Eco → HeatLow`. `change_mode` loops pressing MODE until the desired mode is reached; this order must stay in sync with the physical remote.
- **Power on/off resets state** — both `power_on` and `power_off` reset temperature to 23°C and mode to Eco to mirror what the physical remote does.
- **All unsafe FFI is expected** — the entire Furi/Flipper API surface is `unsafe`. Don't add `unsafe` blocks beyond what's needed to call FFI.
- **`ufmt` not `std::fmt`** — use `uDebug`/`ufmt` derives and macros for formatting; `std::fmt` is unavailable in `no_std`.
