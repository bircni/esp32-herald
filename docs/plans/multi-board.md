# Plan: make Herald easy to port to other ESP32 boards

Goal: adding a board means one small crate (pins, display driver, input,
optional outputs) plus, only if its screen size is new, a layout. Everything
else (WiFi, MQTT, storage, OTA, log, web app, API, feed, weather, custom
screens) is shared and needs no change.

Two boards exist today and must keep working exactly as now:

| Board | Chip | Display | Input | Extras |
|---|---|---|---|---|
| Freenove ESP32 Display 3.5" (FNK0103), `crates/firmware` | ESP32, 4 MB flash, no PSRAM | ST7796 480x320 colour TFT, region redraw in 16k-pixel bands | XPT2046 resistive touch (calibrated) | backlight PWM, RGB LED, speaker |
| Elecrow CrowPanel 5.79" e-paper, `crates/firmware-epaper` | ESP32-S3, 8 MB flash, 8 MB PSRAM | 272x792 (upright) black-and-white e-paper, full frames, partial/full refresh | rotary switch (up/down/press), MENU, EXIT | none |

## Today's coupling (what this plan removes)

- `firmware-epaper` includes the TFT firmware's `logger`, `mqtt`, `net`,
  `shared` and `storage` with `#[path = "../../firmware/src/..."]`, and
  silences what it does not use (`take_beeps`, `calibration`) by hand.
- The two `main.rs` (262 and 170 lines) duplicate start-up, settings and
  time zone handling, restart, heap logging and the loop.
- `ui` assumes one screen: `model::SCREEN_W`/`SCREEN_H` (480x320) and
  `model::screen()` are used by `gfx::render`, `Calibration::fits_screen`,
  `App` and every TFT view (fixed coordinates, about 40 places).
- `App` mixes app state with TFT-only parts: calibration, gesture
  recogniser, menu, the Rgb565 band buffer and region cache, backlight, LED.
  E-paper code reaches into it (`epaper::button`, `App::touched`,
  `shown_frame`).
- The web app switches on `S.epaper` and `tft-only`/`epaper-only` classes.
- CI only checks the firmwares' formatting; nothing builds them.

## Steps

Each step ends with all checks green and both boards verified on hardware
(see Verification). Commit per step.

### 1. Shared firmware library and per-board crates

Layout (one cargo workspace for all firmware, separate from the host
workspace because of the Xtensa toolchain and `build-std`):

```
firmware/
  Cargo.toml              # [workspace] members = ["core", "boards/*"]
  Cargo.lock              # one lock for all boards
  .cargo/config.toml      # linker, build-std, ESP-IDF version, aliases
  rust-toolchain.toml     # esp
  core/                   # lib crate `herald-esp`
    src/{lib,net,mqtt,storage,shared,logger,ota,run}.rs
  boards/
    freenove-35/          # bin `herald-freenove-35`
      Cargo.toml          # [package.metadata.esp-idf-sys] sdkconfig + partitions
      sdkconfig.defaults  # board-specific lines only
      partitions.csv
      src/{main,board,touch,outputs}.rs
    crowpanel-579/        # bin `herald-crowpanel-579`
      ...  src/{main,board,epd,buttons}.rs
  sdkconfig.defaults      # shared lines (stacks, tick, rollback, size options, WiFi notes)
```

- `herald-esp` exposes a `Board` trait and `run(board, peripherals)`:

  ```rust
  pub trait Board {
      /// Size, colour, input and outputs; see step 2.
      fn profile(&self) -> ui::Profile;
      /// Input since the last call (touch samples or button presses), as
      /// step 3's actions or raw samples.
      fn poll_input(&mut self) -> ui::Input;
      /// Shows the app: TFT boards draw changed regions, frame boards
      /// render and refresh (step 4).
      fn present(&mut self, app: &mut ui::App, env: &ui::Env) -> anyhow::Result<()>;
      /// Backlight, LED, beeps; default no-ops for boards without them.
      fn outputs(&mut self, _out: &ui::Outputs) {}
  }
  ```

  `run` owns everything both `main.rs` do today: logger, NVS, storage,
  settings, time zone, `Shared`, `net::spawn`, the loop with its tick,
  effects, restart and the heap/UI timing log line.
- Each board's `main.rs` only takes pins from `Peripherals`, builds its
  `Board` and calls `herald_esp::run`.
- Per-board ESP-IDF config: esp-idf-sys reads
  `[package.metadata.esp-idf-sys] esp_idf_sdkconfig_defaults = ["../../sdkconfig.defaults", "sdkconfig.defaults"]`
  from the root crate, chosen with `ESP_IDF_SYS_ROOT_CRATE`; it also picks
  up `<file>.<chip>` variants by itself. The partition table goes in via
  `CONFIG_PARTITION_TABLE_CUSTOM_FILENAME` in the board's sdkconfig.
  Builds per board via cargo aliases in `.cargo/config.toml`, e.g.
  `cargo freenove` = `build --release -p herald-freenove-35 --target xtensa-esp32-espidf`
  with `ESP_IDF_SYS_ROOT_CRATE` and `MCU` set (an `xtask` or `justfile` if
  aliases cannot set env; keep it to one obvious command per board, plus
  `flash-<board>` and `ota-<board>`).
- Caveat to check: two boards on the same chip share esp-idf-sys's build
  dir for that target, so switching between them rebuilds ESP-IDF. Fine for
  now; note it in the docs.
- Keep: ESP-IDF v5.5.5 (5.5.6 fails to create the WiFi task, see the TFT
  config comment), the size profile, `ESP_IDF_TOOLS_INSTALL_DIR` in one
  place, the lint table (move it to `[workspace.lints]` of the firmware
  workspace), `#[expect(unsafe_code, reason)]` per FFI call.
- NVS namespace, OTA endpoint, rollback marking and the setup access point
  name stay as they are, so boards update over WiFi without a USB flash.
  The partition tables do not change.
- Remove `crates/firmware` and `crates/firmware-epaper`; update the root
  `Cargo.toml` `exclude`, README, `docs/development.md`, CI.

Done when: no `#[path]`, no `dead_code` expects for unused shared code, each
board crate is pins + drivers only, both images build with one command each
and run unchanged.

### 2. Device profile in `ui`, web app driven by capabilities

- Add `ui::Profile` (in `model.rs` or a new `profile.rs`):

  ```rust
  pub struct Profile {
      pub name: &'static str,          // "freenove-35", shown in the web app
      pub width: i32, pub height: i32, // upright, after rotation
      pub color: bool,                 // false: black and white
      pub touch: bool,                 // needs calibration when resistive
      pub buttons: bool,
      pub backlight: bool, pub led: bool, pub speaker: bool,
      pub frame: bool,                 // keeps a full frame: /api/frame.bmp works
  }
  ```

  `App::new(profile, settings, calibration)` stores it. Replace
  `SCREEN_W`/`SCREEN_H`/`screen()` with the profile's size everywhere
  (`gfx::render` clips to a passed rectangle, `Calibration::fits_screen`
  compares with the profile).
- `/api/state` gains `"device": {...profile...}` and drops `"epaper"`; the
  frame revision becomes `"frame": "<rev>"` (present when `frame`).
- Web app: replace `S.epaper`, `tft-only` and `epaper-only` with
  capability classes set from `S.device` (`no-backlight`, `no-touch`,
  `mono`, `frame`, ...). Brightness, sleep and night brightness need
  `backlight`; accent colour needs `color`; tap/swipe on the mirror needs
  `touch`; "Upside down" applies to every board (step 3 makes it work on
  the TFT too). The mirror uses `/api/frame.bmp` whenever `frame` is set,
  else draws in the browser as now (TFT layout).
- Settings stay one struct; nothing board-specific is added to it.

Done when: grepping `ui` and the web app for `480`, `320`, `epaper` outside
layout code finds nothing board-specific; the simulator can start either
profile (`--board freenove-35|crowpanel-579`, replacing `--epaper`).

### 3. One input layer

- Add `ui::Action { Next, Previous, Home, Menu, Refresh, Tap(Point), LongPress(Point) }`
  and `App::act(action, env)`. Rotation pauses, night wake, sleep and
  "a dark screen only wakes" live there, once.
- Touch boards: the gesture recogniser and calibration move into a
  `ui::touch` module that turns raw samples into actions (swipe left =
  Next, long press = Menu, ...). `App` no longer holds `Recognizer`,
  `CalibrateState` or `swallow_touch`; the calibration screen stays a view
  the touch layer shows until calibrated.
- Button boards: buttons map to actions in the board crate (CrowPanel:
  up = Previous, down = Next, press/EXIT = Home, MENU = Refresh).
- `/api/tap`, `/api/swipe` and MQTT remote gestures go through `act`.
- "Upside down" on the TFT: mipidsi orientation set at runtime plus
  mirrored touch mapping, so the setting works on both boards.
- Page order per layout: the TFT steps through home, forecast, calendar,
  screens; the tall layout through dashboard and screens. The layout
  declares its pages; `Next`/`Previous` follow them.

Done when: `epaper::button` and `App::touched` are gone, all input paths
use `act`, and the existing gesture, rotation and calibration tests pass
against the new API (plus tests for button actions).

### 4. Layouts per screen size class, two ways to present

- `ui::layout::Layout` trait: `pages()`, `paint(page, canvas, ...)` and
  `regions(page, ...)` (regions only for region-redraw boards). Two
  implementations: `Landscape480x320` (today's TFT views) and
  `Portrait272x792` (today's `epaper.rs`). The profile picks one by size
  and colour; adding a size class means one new layout.
- Make the TFT views size-independent where cheap: derive positions from
  the canvas size instead of literals, so a 320x240 or 480x272 landscape
  board can reuse `Landscape` with scaled positions and smaller fonts.
  Custom screens already lay out from an `Area`.
- Two presenters in `ui`:
  - `RegionPresenter`: today's banded region redraw for colour TFTs
    without PSRAM (cache keys, band buffer).
  - `FramePresenter`: render the whole frame (colour or 1-bit with the
    current dithering), diff, and hand the board a frame plus refresh
    kind; keeps the last frame for `/api/frame.bmp`. Usable by any board
    with enough RAM, including colour boards with PSRAM.
- Monochrome theme and dithering move from `epaper.rs` into the theme and
  presenter, so any black-and-white board gets them.

Done when: `epaper.rs` is split into the portrait layout and the frame
presenter; the redraw test (incremental equals full) runs for both
layouts; screenshots for both boards are unchanged pixel for pixel (or
the differences are explained).

### 5. CI builds every board, "Adding a board" doc

- CI job `firmware` with a matrix over boards: install the ESP toolchain
  (`espup install --targets esp32,esp32s3` or the `esp-rs/xtensa-toolchain`
  action), cache `firmware/target` and the ESP-IDF tools dir, run
  `cargo fmt --check`, `cargo clippy -D warnings` and the release build
  per board; upload each image (`espflash save-image`) as an artifact, so
  updates can be taken from CI. Expect 10-15 min per board on a cold
  cache.
- `docs/adding-a-board.md`: checklist (board crate from a template, pin
  table, sdkconfig lines, partition table, `Profile`, display driver
  implementing the presenter's output, input to actions, layout only if
  the size is new, simulator profile, CI matrix entry, README table).
- README: a "Boards" table listing supported boards and how to build each.

## Constraints

- Lints: the workspace lint set (pedantic, nursery, restriction list,
  `unsafe_code = "deny"`); fix warnings, allow only with a written reason.
- Edition 2024; dependencies on their latest versions.
- Xtensa LLVM crashes on serde `f32` through untagged enums: keep parsing
  as `f64` and narrowing (`de::number_f32`).
- WiFi buffers and A-MPDU stay at ESP-IDF defaults (smaller ones caused
  multi-second latency).
- No private data in the repo (network names, hosts, IPs, MAC addresses,
  e-mail addresses).
- Commit per step; no pushing without asking.

## Verification (every step)

1. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --release -- -D warnings`,
   `cargo test --workspace --release` (host), and fmt + clippy + release
   build for every board.
2. Simulator screenshots for both boards (`--screenshots`), compared with
   the previous ones.
3. Both boards updated over WiFi (`/api/ota`) and checked: boots, WiFi,
   MQTT connected, retained weather and items shown, custom screen from
   MQTT shown, buttons/touch work, web app mirror and settings page show
   the right controls, a second OTA marks the new slot valid.
4. Hardware details (TFT only): touch calibration still accepted (no
   recalibration forced), backlight, LED, beep on an alert.
