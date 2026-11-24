# Adding a board

Everything that is not about the board's own hardware is shared: WiFi and
setup mode, the web app and API, MQTT, storage, updates over WiFi, the feed,
weather and custom screens. A new board is one crate with its pins and
drivers, a profile, and, only if its screen size is new, a layout.

The two existing boards are the examples to copy from:

| Board | Crate | Display | Input |
|---|---|---|---|
| Freenove ESP32 Display 3.5" | `firmware/boards/freenove-35` | 480x320 colour TFT, region redraw | resistive touch |
| Elecrow CrowPanel 5.79" | `firmware/boards/crowpanel-579` | 272x792 black-and-white e-paper, whole frames | rotary switch, buttons |

## 1. Profile

Add a constant to `ui::Profile` (`crates/ui/src/profile.rs`) and to
`Profile::ALL`: the name (the crate's folder name), the upright screen size,
and what the board has (colour, touch, buttons, backlight, LED, speaker,
whether it keeps whole frames, and with `rotatable` whether its screen can
be used both ways round, which adds an Orientation choice to the web app).
The web app hides controls the board cannot use, and the simulator can run
it with `--board <name>`.

## 2. Layout, only for a new screen size

`ui::layout::Layout::for_profile` picks the layout from the size as the
screen is used: taller than wide gets the portrait layout (272x792), low and
wide (under 300 pixels high) the wide one (792x272), the rest the landscape
one (480x320). A board of one of these sizes needs nothing here.

For another size, add a variant to `Layout` and a module next to
`landscape.rs`, `portrait.rs` and `wide.rs` (the e-paper layouts share
their blocks in `paper.rs`) that paints it (`paint`), says which pages
`Next`/`Previous` step through (`pages`) and, for region redraws, which
regions can change (`regions`). Custom screens lay themselves out in any
area (`views::custom::Area`). Extend `tests/redraw.rs` with the new size.

## 3. Board crate

Copy the closer of the two crates to `firmware/boards/<name>` and change:

- `Cargo.toml`: package and binary name `herald-<name>`, and
  `esp_idf_sdkconfig_defaults` pointing at the board's own file.
- `sdkconfig.defaults`: only what differs from `firmware/sdkconfig.defaults`
  (flash size, PSRAM, console). Each line with a comment why.
- `partitions.csv`: two equal app slots for updates over WiFi, sized for
  the flash.
- `src/board.rs`: the pin table as documentation, `NAME`, timings.
- `src/main.rs`: take the pins from `Peripherals`, build the drivers, and
  implement `herald_esp::Board`:
  - `PROFILE`: the profile from step 1.
  - `TICK`: how often the loop runs (30 ms for touch, 20 ms for buttons).
  - `input`: touch boards call `app.touch(sample, env)`, button boards
    `app.act(action, env)` per press; then `app.tick(env)`.
  - `present`: colour displays without PSRAM use
    `ui::present::RegionPresenter` with any `DrawTarget<Color = Rgb565>`;
    e-paper and other whole-frame displays use `FramePresenter` and convert
    the frame into the controller's memory layout (add that conversion to
    `ui::present::panel`, with a test).
  - `outputs` (optional): backlight, LED, speaker.

`herald_esp::start` and `herald_esp::run` do the rest.

## 4. Build, flash, CI

- `firmware/Makefile`: add the board to `BOARDS` with its Rust target
  (`TARGET_<name>`) and chip (`CHIP_<name>`). Then `make build-<name>`,
  `make flash-<name>`, `make ota-<name> HOST=...` work.
- `.github/workflows/ci.yml`: add the name to the `firmware` job's matrix.
  CI lints and builds it and keeps the image as an artifact.
- If the chip is new to the project, add it to `espup install --targets`
  in the README and to `buildtargets` in CI.

## 5. Docs

Add the board to the README's board table and its hardware to
`docs/development.md`.
