# CLAUDE.md

## What this is

Flipper Zero app (`.fap`) written in `no_std` Rust. A home screen of four icons opens one screen per device: a Daikin air conditioner, a heater and a tower fan over the IR blaster, and two Home Assistant LED bulbs over GPIO. Automatically powers/configures the heater at daytime hours; manual override via OK button.

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

`daikin::self_check` stands in for the missing harness: it runs once from `Daikin::default()` and rebuilds a real ARC-remote capture (IRremoteESP8266's `TestDecodeDaikin.RealExample`) through the setters, reads it back through the getters, and checks the frame length. A layout or checksum mistake panics the app on launch instead of silently transmitting a frame the A/C ignores.

## Architecture

Single binary, event loop in `run()` (`main.rs`). Key modules:

- **`state`** — `AppState` (shared across draw/input callbacks via mutex), `HeaterState` (tracks on/off, temp 5–35°C, mode), `RunState` (state machine: `WaitingForDaytime` → `Changing` → `SetDaytimeHeat`)
- **`daikin`** — the ARC466A33 protocol: the 35-byte remote state, its setters/getters, the three section checksums and the raw timing encoder. Pure protocol, no UI
- **`ac`** — `AcState`: the A/C screen's cursor over a `Daikin`, plus the labels the draw code renders
- **`icons`** — 16x16 XBM bitmaps for the home tiles (regenerate with `tools/xbm.py`)
- **`fan`** — `FanState` for the tower fan (speed, mode, rotation, timer, light). Same open-loop IR model as the heater
- **`bulbs`** — `BulbsState` for the two HA bulbs. Not IR: pulses GPIO pins that an ESP8266 relays to Home Assistant (see below)
- **`ir`** — sends raw IR signals via `infrared_send_raw_ext`. All button timings (POWER, MODE, WARMER, COOLER) are hardcoded pulse arrays captured from the real remote. `ir::fan` holds the fan's NEC frames
- **`notification`** — LED feedback sequences for power on/off and daytime events
- **`allocator`** — thin wrapper around Flipper's `malloc`/`free` as the global allocator (required for `no_std` + `alloc`)

### Control flow

`run()` loop acquires mutex each tick, checks RTC time, fires `start_of_day_power_heater()` once per day (weekdays 08:00–13:00, weekends 09:00–13:00 via `last_daytime_run_day`), then drains the input queue with 100ms timeout.

The app opens straight into the A/C. Back backs out one level — a device screen returns to the home screen, and the home screen has no level left so it exits. Holding Back quits from anywhere, mid-screen included. On home, the arrows walk a 2x2 tile grid (A/C, Bulbs / Fan, Heater) and OK enters a tile. `in_device` on `AppState` is what separates "home cursor" from "screen being shown"; `ActiveDevice::step` does the grid maths by flipping one bit of the tile index.

Inside a device, keys are dispatched per screen:

| Device | OK | Up / Down | Left / Right |
|---|---|---|---|
| A/C | short: power, or the focused row's action — open it if it leads somewhere · long: power on and resend state (both send all 35 bytes) · in a picker: pick | move the cursor down the settings list (repeats while held), or through a picker | change the selected setting, or open the picker on a picker row · hold to run through Temp, Fan and the timers |
| Heater | short: on (Eco, 23°C) · long: daytime (HeatHigh, 35°C) · either when on: off | — | — |
| Fan | short: on · long: on + 1h timer + light off · either when on: off | Up short: speed · Up long: timer · Down short: rotation · Down long: mode | — |
| Bulbs | drives the pair on, or off when both are already on | toggle escritório / quarto | — |

`key_sends` decides which of those actually transmit; everything else (home navigation, walking the A/C list) skips the blue blink and the "Changing..." overlay. The A/C answers for itself through `AcState::sends`, because there it depends on the selected row and on whether the mode picker is open — that predicate lives next to the handlers it describes so the two can't drift apart.

Two rows have named values rather than a number or a flag, so instead of cycling blind they open a picker with an icon per option (`AcState::menu`, `Picker`). OK and the arrows both step into a row that leads somewhere, through the one `AcState::open`; on those rows OK does not toggle power, because a row with a `>` on it that ignored OK read as broken. Holding OK is the exception that answers from anywhere — any row, and inside a picker — and always means the same thing: power on, then send all 35 bytes. It is the way back in sync after the physical remote has been used, so it must not depend on where the cursor sits. Back closes the picker before it closes the screen.

- **Mode** — auto, cool, heat, dry, fan.
- **Run** — normal, eco, power. This one exists because the protocol won't hold eco and powerful at once (`set_powerful` clears econo, `set_econo` clears powerful). As two flag rows they silently switched each other off; as one row with three values the exclusion is the shape of the control rather than a surprise.

### A/C timers

The `Timer` row leads to a sub-list holding `Off at` and `On at`, the protocol's two timers. Two independent times is one too many for a single row, and neither is a choice from a set, so it is a sub-list (`View::Timers`) rather than a [`Picker`] — the cursor in `AcState::field` always points into `view.fields()`. The row itself only says whether anything is armed: `On` or `-`.

Inside are `Off at`, `On at` and `Clear`. Left/Right step a timer in 30-minute notches, holding repeats, and stepping down through zero switches it off. `Clear` acts rather than holding a value, so either arrow triggers it — the same convention the flag rows already use — and it disables both. OK does the same thing there instead of toggling power; `AcState::confirm` holds that decision so it sits next to the rows it depends on. Holding OK still means "on, and resend" everywhere, including there.

Each armed timer shows two lines: the clock time it fires at, and a caption saying how far off that is (`in 2h30`). The stored value is absolute, which is what makes a retransmit idempotent, so the caption is derived at paint time from `daikin::minutes_ahead` rather than stored. That means the sub-list needs its own layout (`draw_timers`) instead of the settings list's plain row loop, and the selection highlight grows to 19px to cover both lines when a timer is armed.

Back unwinds one level at a time — picker, then sub-list, then the screen — through `AcState::go_back`.

The wire format stores an absolute time of day, and so does this — `daikin::next_timer` converts to "minutes from now" only for the duration of a step. That is what makes a retransmit idempotent: if a duration were recomputed on every send, touching an unrelated row would restart the countdown. It also means the displayed value is a fixed clock time rather than a countdown, which is honest about what the A/C was actually told.

Stepping up stops a day out instead of wrapping, since a wrap would silently turn "in 24h" into "in half an hour". The timers survive a restart along with everything else in the saved state, exactly as they survive a battery change in the real remote.

Holding Left or Right runs through the Temp, Fan and timer values. The steps are applied locally and a *single* frame goes out on release (`AcState::repeating`) — every Daikin frame carries the whole state, so only the last one matters, and one send blocks for ~400ms, which would leave the display seconds behind the key. Flag rows don't repeat: there is nothing to run through, and toggling at the repeat rate lands wherever the release happens to fall.

Up/Down repeat too, and need none of that machinery — moving the cursor transmits nothing. The list clamps at its ends, so holding Down means "go to the bottom". The pickers stay tap-only for the same reason the flag rows do: their rings are three and five long, so a held arrow would spin past whatever you were aiming at.

Quiet stays its own flag row: eco and quiet *can* both be on, so folding quiet into Run would lose a valid combination. Powerful does still clear quiet, which is the one cross-row effect left.

The daytime heater automation still only fires while the Heater screen is open, so with the A/C as the opening screen it will not run unless you navigate there.

### A/C: the Daikin protocol

The heater and fan remotes send one frame per button, so their timings are captured. The Daikin is the opposite — it is stateful, and every press retransmits all 35 bytes — so `daikin` builds frames instead of replaying them, ported from IRremoteESP8266's `DAIKIN` protocol (`ir_Daikin.h` lists ARC466A33 under it, not under DAIKIN2/312).

A frame is 583 timings: five bare zero bits, then three header-wrapped sections of 8, 8 and 19 bytes, each ending in its own checksum byte and a 29.4ms gap. Bytes go out LSB-first.

Every edit on the screen sends the whole state immediately, exactly as the physical remote does. Each send stamps the frame with the Flipper's clock (`AcState::send`), because the A/C's own timers run off it — note Furi counts Monday as 1 and the remote counts Sunday as 1.

The 35 bytes are then written to `/ext/apps_data/cold-zero/ac.bin` and reloaded by `AcState::load` on the next launch. This matters more here than for the other devices: because one press sends *everything*, a wrong starting guess doesn't just show wrong, it gets transmitted. `Daikin::from_raw` rejects a file whose three section headers aren't `11 DA 27`, and any storage failure falls back to the default state — a Flipper with no SD card still has to work.

Persistence is not synchronisation. Nothing reads the A/C back, so the physical remote still desyncs the app, and `Hold OK` (resend the app's state) remains the only way to force them back into agreement.

The A/C screen is the only one drawn in `ViewPortOrientationVertical`, so all eleven settings fit on one list. The firmware rotates input keys along with the screen, so handlers keep working in logical directions and need no remapping.

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

- **IR timings are raw pulse arrays** — values are microsecond on/off durations captured from the physical remote. Never synthesise them; always capture from the real device. The Daikin is the one exception, and only because its protocol is fully documented: `daikin` builds frames from the state, and `self_check` pins that construction against a real capture.
- **`HeaterMode::next()` cycles** `HeatLow → HeatHigh → Eco → HeatLow`. `change_mode` loops pressing MODE until the desired mode is reached; this order must stay in sync with the physical remote.
- **Power on/off resets state** — both `power_on` and `power_off` reset temperature to 23°C and mode to Eco to mirror what the physical remote does.
- **Bulb GPIO lines are parked high *before* `furi_hal_gpio_init`** — the output register powers up low, so initialising first drives a falling edge and Home Assistant sees a phantom press the instant the app opens. Don't "tidy" that order in `bulbs::init`. `bulbs::deinit` hands the pins back as analog on exit, leaving the ESP's pullup to hold the released state.
- **Every input handler must gate on `input_event.type_`** — one tap emits `InputTypePress`, `InputTypeShort` and `InputTypeRelease`, so an ungated handler fires three times. This is invisible on a two-item ring (three steps net one move) and obvious on a three-item one.
- **Canvas strings are ASCII, and must be NUL-terminated** — the stock fonts are u8g2 `_tr` variants (glyphs 32–127) and `canvas_draw_str` indexes glyphs per byte. Write accents as single-byte latin-1 escapes (`c"Escrit\xf3rio:"`), never as UTF-8 `\u{f3}`, which would render as two wrong glyphs. Anything built with `format!` needs a trailing `\0`; `String` doesn't have one.
- **Device state is assumed, never read back** — heater, fan and bulbs are all open-loop. Changing a bulb from Home Assistant, or the heater from its own remote, desyncs the display until the next press.
- **All unsafe FFI is expected** — the entire Furi/Flipper API surface is `unsafe`. Don't add `unsafe` blocks beyond what's needed to call FFI.
- **`ufmt` not `std::fmt`** — use `uDebug`/`ufmt` derives and macros for formatting; `std::fmt` is unavailable in `no_std`.
