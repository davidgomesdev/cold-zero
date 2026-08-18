# CLAUDE.md

## What this is

Flipper Zero app (`.fap`) written in `no_std` Rust. Controls three devices from one screen each: a heater and a tower fan over the IR blaster, and two Home Assistant LED bulbs over GPIO. Automatically powers/configures the heater at daytime hours; manual override via OK button.

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
- **`fan`** — `FanState` for the tower fan (speed, mode, rotation, timer, light). Same open-loop IR model as the heater
- **`bulbs`** — `BulbsState` for the two HA bulbs. Not IR: pulses GPIO pins that an ESP8266 relays to Home Assistant (see below)
- **`ir`** — sends raw IR signals via `infrared_send_raw_ext`. All button timings (POWER, MODE, WARMER, COOLER) are hardcoded pulse arrays captured from the real remote. `ir::fan` holds the fan's NEC frames
- **`notification`** — LED feedback sequences for power on/off and daytime events
- **`allocator`** — thin wrapper around Flipper's `malloc`/`free` as the global allocator (required for `no_std` + `alloc`)

### Control flow

`run()` loop acquires mutex each tick, checks RTC time, fires `start_of_day_power_heater()` once per day (weekdays 08:00–13:00, weekends 09:00–13:00 via `last_daytime_run_day`), then drains the input queue with 100ms timeout.

Left/Right cycle the active device (Heater → Fan → Bulbs → Heater); `default_device` picks the opening screen by month. OK, Up and Down are dispatched per active device:

| Device | OK | Up | Down |
|---|---|---|---|
| Heater | short: on (Eco, 23°C) · long: daytime (HeatHigh, 35°C) · either when on: off | — | — |
| Fan | short: on · long: on + 1h timer + light off · either when on: off | short: speed · long: mode | rotation |
| Bulbs | drives the pair on, or off when both are already on | toggle escritório | toggle quarto |

Back → exit app.

### Bulbs: GPIO → ESP8266 → Home Assistant

The bulbs are Zigbee/Matter and live on Home Assistant, so the Flipper cannot reach them directly. Each has a wire to an ESP8266 running ESPHome:

| Bulb | Flipper pin | ESP8266 pin |
|---|---|---|
| Escritório | 2 — PA7 | GPIO5 (D1) |
| Quarto | 3 — PA6 | GPIO4 (D2) |

Which HA entity each one drives lives in the ESPHome YAML on the board, not in this repo — the app only knows "pulse PA7".

Grounds are tied (Flipper pin 8). The ESP powers itself; it must not run off the Flipper's 3V3 pin, which browns out under the radio's ~350mA bursts.

Each ESP pin is an `INPUT_PULLUP` + `inverted: true` binary sensor with `delayed_on_off: 20ms`, and its `on_press` calls `light.toggle`. So **one press = pull the line low for 50ms** (`bulbs::PULSE`). The two halves are a contract: change the debounce on the ESP and `PULSE` has to keep clearing it.

### IR quirk

`set_temp` sends one extra button press (`change_needed.abs() + 1`) because the first warmer/cooler press doesn't register on the physical remote.

### Furi concurrency model

Draw callback (`on_draw`) and input callback (`on_input`) run on different Furi threads. Input callback only enqueues events; all state mutation happens in the main loop under the mutex.

## Key conventions

- **IR timings are raw pulse arrays** — values are microsecond on/off durations captured from the physical remote. Never synthesise them; always capture from the real device.
- **`HeaterMode::next()` cycles** `HeatLow → HeatHigh → Eco → HeatLow`. `change_mode` loops pressing MODE until the desired mode is reached; this order must stay in sync with the physical remote.
- **Power on/off resets state** — both `power_on` and `power_off` reset temperature to 23°C and mode to Eco to mirror what the physical remote does.
- **Bulb GPIO lines are parked high *before* `furi_hal_gpio_init`** — the output register powers up low, so initialising first drives a falling edge and Home Assistant sees a phantom press the instant the app opens. Don't "tidy" that order in `bulbs::init`. `bulbs::deinit` hands the pins back as analog on exit, leaving the ESP's pullup to hold the released state.
- **Every input handler must gate on `input_event.type_`** — one tap emits `InputTypePress`, `InputTypeShort` and `InputTypeRelease`, so an ungated handler fires three times. This is invisible on a two-item ring (three steps net one move) and obvious on a three-item one.
- **Canvas strings are ASCII, and must be NUL-terminated** — the stock fonts are u8g2 `_tr` variants (glyphs 32–127) and `canvas_draw_str` indexes glyphs per byte. Write accents as single-byte latin-1 escapes (`c"Escrit\xf3rio:"`), never as UTF-8 `\u{f3}`, which would render as two wrong glyphs. Anything built with `format!` needs a trailing `\0`; `String` doesn't have one.
- **Device state is assumed, never read back** — heater, fan and bulbs are all open-loop. Changing a bulb from Home Assistant, or the heater from its own remote, desyncs the display until the next press.
- **All unsafe FFI is expected** — the entire Furi/Flipper API surface is `unsafe`. Don't add `unsafe` blocks beyond what's needed to call FFI.
- **`ufmt` not `std::fmt`** — use `uDebug`/`ufmt` derives and macros for formatting; `std::fmt` is unavailable in `no_std`.
