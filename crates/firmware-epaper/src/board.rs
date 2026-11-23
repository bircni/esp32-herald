//! Elecrow `CrowPanel` ESP32 5.79" E-Paper HMI Display (DIS08792E):
//! ESP32-S3-WROOM-1-N8R8 (8 MB flash, 8 MB PSRAM), 792x272 black-and-white
//! e-paper with two SSD1683 controllers, no touch.
//!
//! The pins are taken from `Peripherals` in `main.rs`; this table documents
//! them.
//!
//! | Function            | GPIO | Notes                                |
//! |---------------------|------|--------------------------------------|
//! | E-paper power       | 7    | high switches the panel on           |
//! | E-paper SCK         | 12   |                                      |
//! | E-paper MOSI        | 11   |                                      |
//! | E-paper CS          | 45   |                                      |
//! | E-paper DC          | 46   |                                      |
//! | E-paper reset       | 47   | low resets                           |
//! | E-paper busy        | 48   | high while refreshing                |
//! | EXIT button         | 1    | low when pressed                     |
//! | MENU button         | 2    | low when pressed                     |
//! | Rotary switch down  | 4    | low when pressed                     |
//! | Rotary switch press | 5    | low when pressed                     |
//! | Rotary switch up    | 6    | low when pressed                     |

/// The panel's controllers take up to 20 MHz.
pub const EPD_SPI_HZ: u32 = 10_000_000;
