//! Colour palette derived from the settings.

use embedded_graphics::{pixelcolor::Rgb565, prelude::*};

use crate::model::{ItemColor, Settings, ThemeKind, parse_hex_color};

/// The device's colours: the same values as the web app's mirror.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub bg: Rgb565,
    pub fg: Rgb565,
    /// Secondary text.
    pub dim: Rgb565,
    /// Cards, tiles and button backgrounds.
    pub panel: Rgb565,
    /// Dividers, tracks and inactive navigation dots.
    pub rule: Rgb565,
    /// Highlights: today, the temperature curve, buttons.
    pub accent: Rgb565,
    /// Text on top of `accent`.
    pub on_accent: Rgb565,
    /// Feed and widget colours, the same in both themes.
    pub good: Rgb565,
    pub warn: Rgb565,
    pub caution: Rgb565,
    pub info: Rgb565,
    pub grey: Rgb565,
}

const fn rgb(r: u8, g: u8, b: u8) -> Rgb565 {
    Rgb565::new(r >> 3, g >> 2, b >> 3)
}

impl Theme {
    #[must_use]
    pub fn from_settings(s: &Settings) -> Self {
        let accent = parse_hex_color(&s.accent_color).unwrap_or(rgb(0x35, 0xD0, 0xFF));
        let (bg, fg, dim, panel, rule) = match s.theme {
            ThemeKind::Dark => (
                Rgb565::BLACK,
                rgb(0xF2, 0xF4, 0xF7),
                rgb(0x8D, 0x96, 0xA3),
                rgb(0x1C, 0x21, 0x28),
                rgb(0x2A, 0x30, 0x38),
            ),
            ThemeKind::Light => (
                rgb(0xF3, 0xF4, 0xF1),
                rgb(0x15, 0x19, 0x1F),
                rgb(0x6B, 0x73, 0x7E),
                rgb(0xE2, 0xE6, 0xEA),
                rgb(0xCF, 0xD5, 0xDB),
            ),
        };
        Self {
            bg,
            fg,
            dim,
            panel,
            rule,
            accent,
            on_accent: Rgb565::BLACK,
            good: rgb(0x3D, 0xD6, 0x8C),
            warn: rgb(0xFF, 0x5A, 0x4F),
            caution: rgb(0xFF, 0xC4, 0x2E),
            info: rgb(0x4A, 0x9B, 0xFF),
            grey: rgb(0x8A, 0x8F, 0x98),
        }
    }

    /// For black-and-white displays: black on white, or white on black.
    /// The one grey (gauge tracks) becomes a dot pattern.
    #[must_use]
    pub const fn mono(dark: bool) -> Self {
        let (bg, fg) = if dark {
            (Rgb565::BLACK, Rgb565::WHITE)
        } else {
            (Rgb565::WHITE, Rgb565::BLACK)
        };
        Self {
            bg,
            fg,
            dim: fg,
            panel: bg,
            rule: Rgb565::new(20, 40, 20),
            accent: fg,
            on_accent: bg,
            good: fg,
            warn: fg,
            caution: fg,
            info: fg,
            grey: fg,
        }
    }

    #[must_use]
    pub const fn item(&self, color: ItemColor) -> Rgb565 {
        match color {
            ItemColor::Red => self.warn,
            ItemColor::Yellow => self.caution,
            ItemColor::Blue => self.info,
            ItemColor::Green => self.good,
            ItemColor::Grey => self.grey,
        }
    }

    #[must_use]
    pub fn key(&self) -> i64 {
        i64::from(self.bg.into_storage()) << 16 | i64::from(self.accent.into_storage())
    }
}
