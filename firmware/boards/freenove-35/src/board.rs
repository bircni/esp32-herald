//! Freenove ESP32 Display 3.5" (FNK0103): ESP32-WROOM-32E, 4 MB flash, no
//! PSRAM, ST7796 320x480 SPI panel, XPT2046 resistive touch.
//!
//! This file only documents the wiring; the pins themselves are taken from
//! `Peripherals` in `main.rs`, because the HAL addresses pins by type.
//!
//! | Function            | GPIO | Notes                                  |
//! |---------------------|------|----------------------------------------|
//! | LCD + touch SCK     | 14   | HSPI, shared bus                       |
//! | LCD + touch MOSI    | 13   |                                        |
//! | LCD + touch MISO    | 12   |                                        |
//! | LCD CS              | 15   |                                        |
//! | LCD DC              | 2    |                                        |
//! | LCD reset           | -    | tied to the ESP32 reset line           |
//! | LCD backlight       | 27   | PWM, active high                       |
//! | Touch CS            | 33   |                                        |
//! | Touch IRQ           | 36   | unused, pressure is read over SPI      |
//! | Speaker             | 26   | into the on-board amplifier            |
//! | Amplifier shutdown  | 4    | low = amplifier on                     |
//! | LED red/green/blue  | 22/16/17 | active low                         |

pub const NAME: &str = "Freenove ESP32 Display 3.5\"";

/// The panel fails above this clock (40 MHz is unreliable on this board).
pub const LCD_SPI_HZ: u32 = 26_666_666;
pub const TOUCH_SPI_HZ: u32 = 2_000_000;

use mipidsi::options::Rotation;

/// The UI is landscape. `Deg90` and `Deg270` put the USB port on opposite
/// sides; switch if the picture is upside down for how the board stands.
/// The touch calibration screen comes back on its own after a change of
/// layout, but not after flipping by 180 degrees: use "Calibrate touch" in
/// the menu then.
pub const LCD_ROTATION: Rotation = Rotation::Deg90;
/// Set if text appears mirrored.
pub const LCD_MIRRORED: bool = true;
/// Set if colours look like a photo negative.
pub const LCD_INVERTED: bool = false;
/// Set if red and blue are swapped.
pub const LCD_BGR: bool = true;

/// Touch pressure (0..=4095 scale, higher = firmer) that counts as a press.
pub const TOUCH_PRESSURE_MIN: u16 = 350;
