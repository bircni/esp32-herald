# Changelog

All notable changes to Herald. The version is the same for the host crates
(`Cargo.toml`) and the firmware (`firmware/Cargo.toml`); the device shows it
on the web app's System page and in `<prefix>/state/<hostname>/state`.

## [0.1.0] - 2026-11-24

The first release.

### Boards

- Freenove ESP32 Display 3.5" (`freenove-35`): 480x320 colour TFT with touch.
- Elecrow CrowPanel ESP32 5.79" E-Paper (`crowpanel-579`): black-and-white
  e-paper, upright or landscape, with rotary switch and buttons.

### Features

- Feed of short items over MQTT or HTTP, most urgent first, with colours,
  icons and how long each state has lasted.
- Weather and forecast pushed from Home Assistant, and a calendar view.
- Custom screens from 11 widget types, which join the rotation.
- Web app with a live mirror, a screen builder, settings, logs and firmware
  updates over WiFi.
- Night mode, auto-rotation, a setup page that opens by itself on your
  phone, English and German on the displays.
- MQTT topics split into `<prefix>/data/...` (what to show),
  `<prefix>/cmd/...` (commands) and `<prefix>/state/<hostname>/...` (what
  each display reports, only when it changes).

### Downloads

Per board, two images:

- `herald-<board>-0.1.0-full.bin`: for the first install over USB. It also
  erases the settings: `espflash write-bin 0x0 herald-<board>-0.1.0-full.bin`.
- `herald-<board>-0.1.0-update.bin`: for a board already running Herald,
  uploaded on the web app's System page.

[0.1.0]: https://github.com/bircni/esp32-herald/releases/tag/v0.1.0
