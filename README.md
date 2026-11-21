# esp32-deskbuddy

Custom Rust firmware that turns a Freenove ESP32 Display 3.5" into a desk
dashboard in landscape: a clock and the weather on top, and below them a
feed of short items that other machines push to it (an open window, Claude
Code waiting for input, a failed build). Swipe sideways for an hourly and
four-day forecast and a calendar page, or let the views rotate on their own.

> **Status:** runs on the real board. Everything that can run without it is
> tested (see [Testing](#testing)).

## Hardware

Freenove ESP32 Display 3.5" (FNK0103): ESP32-WROOM-32E, 4 MB flash, no PSRAM,
ST7796 320x480 SPI panel, XPT2046 resistive touch, CH340 USB serial. The wiring
is documented in [`crates/firmware/src/board.rs`](crates/firmware/src/board.rs).

A speaker (8 Ω, 1 W, PH1.25 plug) is optional; it is only used for beeps.

## Layout

| Path | What it is |
|---|---|
| `crates/ui` | All logic and rendering, hardware-free: views, gestures, feed, rotation, weather parsing, HTTP API, web page |
| `crates/simulator` | Runs `ui` in a desktop window and serves the API on localhost |
| `crates/firmware` | ESP32 target: display, touch, WiFi, storage, speaker, LED. Built separately from the workspace |
| `tools/claude-hook.sh` | Example: show Claude Code's state on the buddy |

## Try it without the board

Needs Rust and SDL2 (`brew install sdl2`).

```sh
LIBRARY_PATH=/opt/homebrew/lib cargo run -p simulator
```

A 480x320 window opens; the mouse is the finger. The same web page and API the
device serves are at <http://127.0.0.1:8080>.

- Swipe left/right: next/previous view (home, forecast, calendar).
- Press and hold for a second: menu (brightness, theme, auto-rotate, touch
  calibration, WiFi).
- `--demo` fills the feed with sample items and sets a weather location,
  `--calibrate` starts with the touch calibration screen, `--setup` in WiFi
  setup mode, `--screenshots DIR` renders every screen to PNG without a window.

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
cargo run --release        # builds, flashes and opens the serial monitor
```

The first build downloads and compiles ESP-IDF into `crates/firmware/.embuild`
and takes several minutes. If the board is not found, hold BOOT, tap RESET,
release BOOT, and try again; on macOS the CH340 shows up as
`/dev/cu.usbserial-*` or `/dev/cu.wchusbserial*`.

## First boot on real hardware

1. **Picture check.** The calibration screen should appear upright in
   landscape, white text on black. If not, change the `LCD_*` constants in
   `board.rs`: upside down → `LCD_ROTATION` (`Deg90` ↔ `Deg270`), mirrored
   text → `LCD_MIRRORED`, negative colours → `LCD_INVERTED`, red and blue
   swapped → `LCD_BGR`.
2. **Touch calibration.** Press and hold each of the four crosses. If presses
   are not noticed, lower `TOUCH_PRESSURE_MIN` in `board.rs`.
3. **WiFi setup.** The display shows a network name (`Buddy-XXXX`) and a
   password. Join it, open the address shown (usually <http://192.168.71.1>), enter your 2.4 GHz WiFi under
   "WiFi". The device restarts and connects. A tap skips setup and uses the
   device offline.
4. **Settings.** The menu (long press) shows the device's address. Open it in a
   browser to set time zone, weather location, colours, rotation and an
   access token.

The serial log prints free heap and UI frame times once a minute. Without PSRAM this is the number
to watch: it should stay well above 60 KB with WiFi connected.

## Views and rotation

| View | Shows |
|---|---|
| Home | Clock, date, current weather, and the feed (up to five rows, most urgent first) |
| Forecast | Temperature curve and rain chance for the next 12 hours, cards for four days |
| Calendar | Weekday and date, ISO week, the month grid, sunrise, sunset and daylight |

With rotation on, the device moves to the next view every `rotate_secs`
seconds, through the views listed in `rotate_views`, between `rotate_from` and
`rotate_until` (local time; equal values mean all day, and the window may
wrap past midnight). It pauses for a minute after any touch. Rotation can be
switched on in the menu; interval, views and hours are set on the web page.

## HTTP API

Plain HTTP on the local network. If an access token is set, every request
except `GET /` and `GET /api/state` needs the header `X-Buddy-Token`. The token
guards against accidents on a shared network; it is not encryption.

| Request | Body | Effect |
|---|---|---|
| `GET /api/state` | | Everything the device is doing right now |
| `GET /api/settings` | | Current settings |
| `PUT /api/settings` | any subset of the settings | Validates and saves |
| `PUT /api/items/<id>` | `{"text","color","detail","age","ttl","wake","beep"}`, only `text` required | Adds or replaces a feed item |
| `DELETE /api/items/<id>` | | Removes it |
| `GET /api/items` | | The feed |
| `DELETE /api/items` | | Empties the feed |
| `POST /api/view` | `{"view":"home"\|"forecast"\|"calendar"}` | Switches view and wakes the display |
| `POST /api/wifi` | `{"ssid","password"}` | Saves credentials and restarts |

Feed items:

- `id`: up to 32 of `a-z A-Z 0-9 - _ .`, e.g. `window.kitchen`. Sending the
  same id again replaces the item.
- `color`: `red`, `yellow`, `blue` (default), `green` or `grey`. Items are
  listed in that order. A red or yellow item at the top lights the LED.
- `detail`: up to 12 characters shown on the right. Without it the right
  side shows how long the item has been in its state (`now`, `12m`,
  `1h 05m`, `2d 3h`). Re-sending the same text and colour keeps that age;
  `age` (seconds) sets it explicitly.
- `ttl`: seconds until the item removes itself (default: never, at most a
  week). The feed holds up to 16 items and lives in RAM only.
- `wake`: switch the display on and show the home view. `beep`: also beep.

```sh
curl -X PUT http://192.168.1.42/api/items/window.kitchen \
  -d '{"text":"Kitchen window open","color":"red","wake":true}'
curl -X DELETE http://192.168.1.42/api/items/window.kitchen
```

### Claude Code hook

`tools/claude-hook.sh working|waiting|done|clear` keeps a `claude` item in the
feed up to date (`BUDDY_ITEM` changes the id, e.g. one per machine). To wire
it up, add hooks to `~/.claude/settings.json` (adjust the path and URL):

```json
{
  "hooks": {
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "BUDDY_URL=http://192.168.1.42 /path/to/tools/claude-hook.sh working" }] }],
    "Notification":     [{ "hooks": [{ "type": "command", "command": "BUDDY_URL=http://192.168.1.42 /path/to/tools/claude-hook.sh waiting" }] }],
    "Stop":             [{ "hooks": [{ "type": "command", "command": "BUDDY_URL=http://192.168.1.42 /path/to/tools/claude-hook.sh done" }] }]
  }
}
```

The script gives up after three seconds and always exits successfully, so a
switched-off buddy does not slow Claude Code down.

## Testing

```sh
cargo test -p ui --release       # logic, API, weather parsing, redraw check
                                 # (the redraw check is slow without --release)
cargo clippy --workspace --all-targets
cd crates/firmware && cargo clippy --release
```

`crates/ui/tests/redraw.rs` drives the app through every screen and checks
after each tick that the partially repainted display equals a full repaint.
The device has no room for a frame buffer, so this is what catches stale
pixels.

The firmware can be booted in Espressif's QEMU, which has no WiFi radio and no
touch panel, so a special build skips both:

```sh
cd crates/firmware
cargo build --release --features qemu
espflash save-image --chip esp32 --merge --flash-size 4mb \
  --partition-table partitions.csv \
  target/xtensa-esp32-espidf/release/deskbuddy-firmware flash.bin
qemu-system-xtensa -nographic -machine esp32 \
  -drive file=flash.bin,if=mtd,format=raw -serial mon:stdio
```

This proves boot, partition table, driver setup and the UI loop on the real
CPU architecture. It cannot show the picture or test touch and WiFi.

## Design notes

- **No frame buffer.** 480x320 at 16 bit is 307 KB, more than the board can
  spare next to WiFi. Each view declares the regions that can change and a key
  describing their content; only regions whose key changed are repainted,
  through a 32 KB band buffer, so animations do not flicker.
- **Weather over plain HTTP.** TLS costs roughly 40 KB of heap. The request
  carries only coordinates, so Open-Meteo is fetched without it. One request
  every 15 minutes brings current values, 12 hours and 4 days (about 1.5 KB).
- **SPI with DMA.** Without it the ESP-IDF driver sends 64 bytes per
  transaction and a full repaint takes over a second.
- **WiFi changes restart the device.** The network thread reads credentials
  once at boot, which keeps the reconnect logic simple.
- **`f32` and serde.** Deriving `Deserialize` for an `f32` field crashes the
  Xtensa LLVM backend. Such fields go through `model::de_f32`.
