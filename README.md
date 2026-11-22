# Herald

[![CI](https://github.com/bircni/esp32-herald/actions/workflows/ci.yml/badge.svg)](https://github.com/bircni/esp32-herald/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A small status display for your desk. Herald is Rust firmware for the
Freenove ESP32 Display 3.5" that shows a clock, the weather and a feed of what
your other systems tell it: an open window, Claude Code waiting for input, a
failed build. It fetches nothing on its own; Home Assistant or any other MQTT
client sends it what to show.

<p align="center">
  <img src="docs/images/home.png" width="480" alt="The home view: a large clock, the weather, and a feed of open windows and build results">
</p>

| Forecast | Calendar | A custom screen |
|---|---|---|
| ![Hourly curve with rain chances and four day cards](docs/images/forecast.png) | ![Today, the month grid, sunrise and sunset](docs/images/calendar.png) | ![Energy screen with value, gauge, chart, list, progress bar and countdown widgets](docs/images/screen-energy.png) |

![The web app: a live mirror of the device's screen with view buttons, brightness and status](docs/images/web-overview.png)

## Features

- **Feed** of short items over MQTT or HTTP, most urgent first, with colours,
  icons and how long each state has lasted.
- **Weather and forecast** pushed from Home Assistant, plus a calendar view.
- **Custom screens** from 11 widget types (values, gauges, charts, lists,
  countdowns, …), sent as retained MQTT messages; they join the rotation.
- **Web app** with a live mirror you can tap, a screen builder, settings,
  logs and firmware updates over WiFi.
- **Night mode**, auto-rotation, a setup page that opens by itself on your
  phone, and Home Assistant-friendly messages (numbers may be strings,
  errors are reported on MQTT).

## Quick start

Try it on your computer first (needs Rust and SDL2, e.g. `brew install sdl2`):

```sh
LIBRARY_PATH=/opt/homebrew/lib cargo run -p simulator -- --demo
```

A 480x320 window opens (the mouse is your finger) and the web app runs at
<http://127.0.0.1:8080>.

To flash the board:

```sh
cargo install espup espflash ldproxy && espup install --targets esp32
. ~/export-esp.sh && cd crates/firmware && cargo build --release
espflash flash --partition-table partitions.csv --erase-parts otadata \
  target/xtensa-esp32-espidf/release/herald-firmware
```

On first boot, tap the four calibration crosses, then join the `Herald-XXXX`
network with your phone and enter your WiFi. Later updates go over WiFi from
the web app.

## Sending things

Set your MQTT broker in the web app, then publish under the topic prefix
(`herald` by default):

| Topic | Payload |
|---|---|
| `herald/item/<id>` | `Front door open`, or JSON with `text`, `color`, `icon`, `since`, `ttl`, …; empty removes it |
| `herald/screen/<name>` | a custom screen: title and widgets |
| `herald/weather` | current weather and forecast, e.g. from a Home Assistant weather entity |
| `herald/view` | `home`, `forecast`, `calendar` or a screen name |

```sh
mosquitto_pub -h broker.lan -r -t herald/item/window.kitchen \
  -m '{"text":"Kitchen window open","color":"red","icon":"window"}'
```

The web app's Connections page lists every topic with your prefix filled in.

## Documentation

- [Using Herald](docs/guide.md): first boot, the web app, views, night mode,
  custom screens and all widgets.
- [MQTT and HTTP](docs/mqtt-and-http.md): every topic and field, weather,
  Home Assistant automations, the HTTP API and the Claude Code hook.
- [Development](docs/development.md): hardware, project layout, building,
  testing and design notes.

## License

[MIT](LICENSE)
