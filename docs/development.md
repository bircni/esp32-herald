# Development

Building, flashing, testing, and why things are the way they are.

## Hardware

Freenove ESP32 Display 3.5" (FNK0103): ESP32-WROOM-32E, 4 MB flash, no PSRAM,
ST7796 320x480 SPI panel, XPT2046 resistive touch, CH340 USB serial. The wiring
is documented in [`crates/firmware/src/board.rs`](../crates/firmware/src/board.rs).

A speaker (8 Ω, 1 W, PH1.25 plug) is optional; it is only used for beeps.

## Layout

| Path | What it is |
|---|---|
| `crates/ui` | All logic and rendering, hardware-free: views, custom screens, gestures, feed, rotation, night mode, weather, HTTP API, MQTT, the web app |
| `crates/simulator` | Runs `ui` in a desktop window (or headless) and serves the API on localhost |
| `crates/firmware` | ESP32 target: display, touch, WiFi, storage, OTA, speaker, LED. Built separately from the workspace |
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

## Build and flash the firmware

One-time setup of the ESP toolchain:

```sh
cargo install espup espflash ldproxy
espup install --targets esp32
```

Then, in every new shell:

```sh
. ~/export-esp.sh
cd crates/firmware
cargo build --release
espflash flash --partition-table partitions.csv --erase-parts otadata \
  target/xtensa-esp32-espidf/release/herald-firmware
```

`--erase-parts otadata` makes the device boot what was just written even if
it last ran an update in the other slot. The first build downloads and
compiles ESP-IDF into `crates/firmware/.embuild` and takes several minutes.
If the board is not found, hold BOOT, tap RESET, release BOOT and try again;
on macOS the CH340 shows up as `/dev/cu.usbserial-*`.

After the first USB flash, updates go over WiFi: on the web app's System page,
or

```sh
curl --data-binary @target/xtensa-esp32-espidf/release/herald-firmware.bin \
  -H 'Content-Type: application/octet-stream' http://herald/api/ota
```

with the image from `espflash save-image --chip esp32
target/xtensa-esp32-espidf/release/herald-firmware herald-firmware.bin`. The
device writes it into the other slot and restarts. If the new firmware never
connects to WiFi, the bootloader goes back to the previous one.

## Testing

```sh
LIBRARY_PATH=/opt/homebrew/lib cargo test --workspace --release
LIBRARY_PATH=/opt/homebrew/lib cargo clippy --workspace --all-targets
cd crates/firmware && . ~/export-esp.sh && cargo clippy --release
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

The lints are the strict set used across these projects (`clippy::pedantic`,
`nursery` and selected restriction lints, `unsafe_code` denied); the firmware's
ESP-IDF calls carry `#[expect(unsafe_code, reason = ...)]` one by one.

The firmware can also be booted in Espressif's QEMU, which has no WiFi radio
and no touch panel, so a special build skips both:

```sh
cd crates/firmware
cargo build --release --features qemu
espflash save-image --chip esp32 --merge --flash-size 4mb \
  --partition-table partitions.csv \
  target/xtensa-esp32-espidf/release/herald-firmware flash.bin
qemu-system-xtensa -nographic -machine esp32 \
  -drive file=flash.bin,if=mtd,format=raw -serial mon:stdio
```

## Design notes

- **No frame buffer.** 480x320 at 16 bit is 307 KB, more than the board can
  spare next to WiFi. Each view declares the regions that can change and a key
  describing their content; only regions whose key changed are repainted,
  through a 32 KB band buffer.
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
