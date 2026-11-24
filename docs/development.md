# Development

Building, flashing, testing, and why things are the way they are.

## Hardware

Freenove ESP32 Display 3.5" (FNK0103): ESP32-WROOM-32E, 4 MB flash, no PSRAM,
ST7796 320x480 SPI panel, XPT2046 resistive touch, CH340 USB serial. The wiring
is documented in [`firmware/boards/freenove-35/src/board.rs`](../firmware/boards/freenove-35/src/board.rs).

A speaker (8 Ω, 1 W, PH1.25 plug) is optional; it is only used for beeps.

Elecrow CrowPanel ESP32 5.79" E-Paper HMI Display: ESP32-S3-WROOM-1-N8R8 (8 MB
flash, 8 MB PSRAM), 792x272 black-and-white e-paper with two SSD1683
controllers, a rotary switch and MENU/EXIT buttons, no touch. Used upright
(272x792). Wiring in
[`firmware/boards/crowpanel-579/src/board.rs`](../firmware/boards/crowpanel-579/src/board.rs).

## Layout

| Path | What it is |
|---|---|
| `crates/ui` | All logic and rendering, hardware-free: views, custom screens, gestures, feed, rotation, night mode, weather, HTTP API, MQTT, the web app |
| `crates/simulator` | Runs `ui` in a desktop window (or headless) and serves the API on localhost |
| `firmware/core` | `herald-esp`, shared by every board: WiFi and setup mode, the HTTP server, MQTT, storage, OTA, the log and the main loop |
| `firmware/boards/<board>` | One crate per board: pins, display driver, input and outputs, its `sdkconfig.defaults` and partition table |
| `firmware/Makefile` | One command per board: `make build-<board>`, `clippy-`, `flash-`, `image-`, `ota-` |
| `tools/claude-hook.sh` | Example: show Claude Code's state on the display |

## Try it without the board

Needs Rust and SDL2 (`brew install sdl2`).

```sh
LIBRARY_PATH=/opt/homebrew/lib cargo run -p simulator -- --demo
```

A 480x320 window opens; the mouse is the finger. The same web app and API the
device serves are at <http://127.0.0.1:8080>.

- Swipe left/right: next/previous view. Press and hold: the menu.
- `--demo` adds sample items, weather and two custom screens,
  `--calibrate` starts with touch calibration, `--setup` in WiFi setup mode,
  `--headless` runs without a window, `--port N` picks the port,
  `--screenshots DIR` renders every screen to PNG.
- `--board crowpanel-579` simulates the e-paper board instead (default
  `freenove-35`, names as in `ui::Profile`): arrow keys are the rotary
  switch, Enter presses it, Escape is EXIT and M is MENU.

## Build and flash the firmware

One-time setup of the ESP toolchain:

```sh
cargo install espup espflash ldproxy
espup install --targets esp32
```

Then, in every new shell:

```sh
. ~/export-esp.sh
cd firmware
make flash-freenove-35 PORT=/dev/cu.usbserial-10   # or flash-crowpanel-579
```

The firmware is a cargo workspace of its own (`firmware/`), apart from the
host workspace, because it needs the Xtensa toolchain and `build-std`. The
Makefile passes each board's Rust target and sets `ESP_IDF_SYS_ROOT_CRATE`,
which tells esp-idf-sys whose `sdkconfig.defaults` to use (the shared
`firmware/sdkconfig.defaults` plus the board's own). Two boards on the same
chip share ESP-IDF's build output, so switching between them rebuilds it.

Flashing passes `--erase-parts otadata`, so the device boots what was just
written even if it last ran an update in the other slot. The first build
downloads and compiles ESP-IDF into `firmware/.embuild` and takes several
minutes.
If the board is not found, hold BOOT, tap RESET, release BOOT and try again;
on macOS the CH340 shows up as `/dev/cu.usbserial-*`.

After the first USB flash, updates go over WiFi: on the web app's System page
(with the image from `make image-<board>`, in `firmware/target/<board>.bin`),
or

```sh
make ota-freenove-35 HOST=herald.local TOKEN=...   # TOKEN only if one is set
```

The device writes it into the other slot and restarts. If the new firmware never
connects to WiFi, the bootloader goes back to the previous one.

## Testing

```sh
LIBRARY_PATH=/opt/homebrew/lib cargo test --workspace --release
LIBRARY_PATH=/opt/homebrew/lib cargo clippy --workspace --all-targets
cd firmware && . ~/export-esp.sh && make clippy-freenove-35 clippy-crowpanel-579
```

- Unit tests in `crates/ui` cover parsing, validation, the API, MQTT topics,
  rotation, night mode, the captive portal's DNS answers and more.
- `crates/ui/tests/redraw.rs` drives the app through every screen and widget
  and checks after each tick that the partially repainted display equals a
  full repaint. The device has no room for a frame buffer, so this is what
  catches stale pixels. It is slow without `--release`.
- `crates/simulator/tests/e2e.rs` starts the simulator headless and uses it
  over HTTP like the web app and Home Assistant do: the gzipped page and
  `ETag`s, items, weather, screens, settings, gestures and the log.

CI runs the same host checks, and for every board in `firmware/boards` the
firmware lints and release build (`make clippy-<board> image-<board>`); the
images are kept as build artifacts, ready for the web app's update page.

The lints are the strict set used across these projects (`clippy::pedantic`,
`nursery` and selected restriction lints, `unsafe_code` denied); the firmware's
ESP-IDF calls carry `#[expect(unsafe_code, reason = ...)]` one by one.

The firmware can also be booted in Espressif's QEMU, which has no WiFi radio
and no touch panel, so a special build skips both:

```sh
cd firmware
ESP_IDF_SYS_ROOT_CRATE=herald-freenove-35 cargo build --release \
  -p herald-freenove-35 --target xtensa-esp32-espidf --features qemu
espflash save-image --chip esp32 --merge --flash-size 4mb \
  --partition-table boards/freenove-35/partitions.csv \
  target/xtensa-esp32-espidf/release/herald-freenove-35 flash.bin
qemu-system-xtensa -nographic -machine esp32 \
  -drive file=flash.bin,if=mtd,format=raw -serial mon:stdio
```

## Design notes

- **A profile, a layout, a presenter.** A board describes itself with a
  `ui::Profile` (size, colour, touch, buttons, backlight, ...). The size picks
  a `ui::layout::Layout` (landscape 480x320, portrait 272x792, wide 792x272;
  rotatable screens turn with the `orientation` setting), and the board
  shows it with one of two `ui::present` presenters. Input of every kind
  becomes a `ui::Action`.
- **No frame buffer on the TFT.** 480x320 at 16 bit is 307 KB, more than the
  ESP32 can spare next to WiFi. Each view declares the regions that can
  change and a key describing their content; the region presenter repaints
  only regions whose key changed, through a 32 KB band buffer.
- **Whole frames on e-paper.** The frame presenter draws the whole picture
  (into PSRAM), dithers it to black and white, and compares it with what the
  panel shows: unchanged frames cost nothing, small changes are partial
  refreshes, new pages and every 30th change full ones.
- **The browser does the work.** The web app is one gzipped page served with
  an `ETag`; it draws the mirror and previews from JSON, and polls a state that
  answers `304` when nothing changed. The device mostly answers small JSON.
- **Weather is pushed, not fetched.** The device makes no outgoing HTTP
  requests (TLS alone would cost about 40 KB of heap).
- **Custom screens live on the broker.** They are retained MQTT messages, so
  they survive restarts without flash writes; a screen made in the web app is
  published retained by the device.
- **Updates over WiFi.** Two 1.94 MB app slots (the firmware is about
  1.5 MB); rollback if a new version never connects.
- **Firmware size.** Measured on the flashed image: `lto = "fat"` with one
  codegen unit saved 73 KB, `opt-level = "z"` another 50 KB, ESP-IDF's nano
  printf, silent assertions and no IPv6 another 98 KB: 1.73 MB → 1.51 MB.
- **WiFi buffers stay at ESP-IDF's defaults.** Cutting them and turning off
  A-MPDU to save RAM made access points queue frames for seconds.
- **UI on the second core.** The main task, which runs the UI loop, is pinned
  to core 1, away from WiFi and TCP/IP on core 0; modem power saving is off.
- **Host name.** Sent to the router over DHCP, so a router that registers DHCP
  names resolves it under the local domain. mDNS answers `<hostname>.local`
  as well (ESP-IDF's `espressif/mdns` component).
- **SPI with DMA.** Without it the ESP-IDF driver sends 64 bytes per
  transaction and a full repaint takes over a second.
- **`f32` and serde.** The Xtensa LLVM backend cannot compile serde's `f32`
  path through untagged enums; such numbers are parsed as `f64` and narrowed
  (`de::number_f32`).
