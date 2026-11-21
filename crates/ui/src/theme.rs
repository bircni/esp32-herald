//! Colour palette derived from the settings.

use embedded_graphics::{pixelcolor::Rgb565, prelude::*};

use crate::model::{parse_hex_color, ItemColor, Settings, ThemeKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub bg: Rgb565,
    pub fg: Rgb565,
    /// Secondary text.
    pub dim: Rgb565,
    /// Button and track background.
    pub panel: Rgb565,
    /// Highlights: today, the temperature curve, buttons.
    pub accent: Rgb565,
    /// Text on top of `accent`.
    pub on_accent: Rgb565,
    pub good: Rgb565,
    pub warn: Rgb565,
    /// Yellow feed items.
    pub caution: Rgb565,
    /// Blue feed items, rain.
    pub info: Rgb565,
}

const fn rgb(r: u8, g: u8, b: u8) -> Rgb565 {
    Rgb565::new(r >> 3, g >> 2, b >> 3)
}

impl Theme {
    pub fn from_settings(s: &Settings) -> Self {
        let accent = parse_hex_color(&s.accent_color).unwrap_or(rgb(0x35, 0xD0, 0xFF));
        match s.theme {
            ThemeKind::Dark => Self {
                bg: Rgb565::BLACK,
                fg: rgb(0xF2, 0xF2, 0xF2),
                dim: rgb(0x8A, 0x8F, 0x98),
                panel: rgb(0x26, 0x2A, 0x30),
                accent,
                on_accent: Rgb565::BLACK,
                good: rgb(0x3D, 0xD6, 0x8C),
                warn: rgb(0xFF, 0x5A, 0x4F),
                caution: rgb(0xFF, 0xC4, 0x2E),
                info: rgb(0x4A, 0x9B, 0xFF),
            },
            ThemeKind::Light => Self {
                bg: rgb(0xF4, 0xF1, 0xEA),
                fg: rgb(0x1B, 0x1D, 0x21),
                dim: rgb(0x6B, 0x70, 0x78),
                panel: rgb(0xDD, 0xD8, 0xCE),
                accent,
                on_accent: Rgb565::BLACK,
                good: rgb(0x1E, 0x9E, 0x62),
                warn: rgb(0xD9, 0x3A, 0x30),
                caution: rgb(0xD8, 0x9A, 0x00),
                info: rgb(0x1F, 0x6F, 0xD6),
            },
        }
    }

    pub fn item(&self, color: ItemColor) -> Rgb565 {
        match color {
            ItemColor::Red => self.warn,
            ItemColor::Yellow => self.caution,
            ItemColor::Blue => self.info,
            ItemColor::Green => self.good,
            ItemColor::Grey => self.dim,
        }
    }

    pub fn key(&self) -> i64 {
        i64::from(self.bg.into_storage()) << 16 | i64::from(self.accent.into_storage())
    }
}
