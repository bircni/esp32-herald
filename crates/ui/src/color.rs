//! Colours as other machines send them: a name, `#RRGGBB` or `[r, g, b]`.

use std::fmt;

use embedded_graphics::pixelcolor::Rgb565;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::{model::ItemColor, theme::Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Red,
    Yellow,
    Blue,
    Green,
    Grey,
    /// The theme's text colour.
    White,
    /// The accent colour from the settings.
    Accent,
    Rgb(u8, u8, u8),
}

impl Color {
    const NAMES: [(&'static str, Self); 8] = [
        ("red", Self::Red),
        ("yellow", Self::Yellow),
        ("blue", Self::Blue),
        ("green", Self::Green),
        ("grey", Self::Grey),
        ("gray", Self::Grey),
        ("white", Self::White),
        ("accent", Self::Accent),
    ];

    /// A name, or `#RRGGBB` (also without `#`, and `#RGB`).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let lower = s.to_ascii_lowercase();
        if let Some((_, c)) = Self::NAMES.iter().find(|(n, _)| *n == lower) {
            return Some(*c);
        }
        let hex = s.strip_prefix('#').unwrap_or(s);
        let channel = |i: usize, len: usize| u8::from_str_radix(hex.get(i..i + len)?, 16).ok();
        match hex.len() {
            6 => Some(Self::Rgb(channel(0, 2)?, channel(2, 2)?, channel(4, 2)?)),
            3 => {
                let (r, g, b) = (channel(0, 1)?, channel(1, 1)?, channel(2, 1)?);
                Some(Self::Rgb(r * 17, g * 17, b * 17))
            }
            _ => None,
        }
    }

    #[must_use]
    pub const fn rgb565(self, theme: &Theme) -> Rgb565 {
        match self {
            Self::Red => theme.warn,
            Self::Yellow => theme.caution,
            Self::Blue => theme.info,
            Self::Green => theme.good,
            Self::Grey => theme.grey,
            Self::White => theme.fg,
            Self::Accent => theme.accent,
            Self::Rgb(r, g, b) => Rgb565::new(r >> 3, g >> 2, b >> 3),
        }
    }

    /// The feed colour closest to this one; feed colours also set the order
    /// of items and the LED.
    #[must_use]
    pub fn item(self) -> ItemColor {
        match self {
            Self::Red => ItemColor::Red,
            Self::Yellow => ItemColor::Yellow,
            Self::Blue | Self::Accent => ItemColor::Blue,
            Self::Green => ItemColor::Green,
            Self::Grey | Self::White => ItemColor::Grey,
            Self::Rgb(r, g, b) => nearest(r, g, b),
        }
    }

    /// Stable number for redraw keys.
    #[must_use]
    pub fn key(self) -> i64 {
        match self {
            Self::Rgb(r, g, b) => i64::from(r) << 16 | i64::from(g) << 8 | i64::from(b),
            named @ (Self::Red
            | Self::Yellow
            | Self::Blue
            | Self::Green
            | Self::Grey
            | Self::White
            | Self::Accent) => {
                -1 - Self::NAMES
                    .iter()
                    .position(|(_, c)| *c == named)
                    .unwrap_or(0) as i64
            }
        }
    }
}

/// Reference points for mapping any colour onto the five feed colours.
fn nearest(r: u8, g: u8, b: u8) -> ItemColor {
    const REFS: [(ItemColor, [i32; 3]); 5] = [
        (ItemColor::Red, [0xFF, 0x5A, 0x4F]),
        (ItemColor::Yellow, [0xFF, 0xC4, 0x2E]),
        (ItemColor::Blue, [0x4A, 0x9B, 0xFF]),
        (ItemColor::Green, [0x3D, 0xD6, 0x8C]),
        (ItemColor::Grey, [0x8A, 0x8F, 0x98]),
    ];
    let c = [i32::from(r), i32::from(g), i32::from(b)];
    REFS.iter()
        .min_by_key(|(_, p)| (0..3).map(|i| (c[i] - p[i]).pow(2)).sum::<i32>())
        .map_or(ItemColor::Grey, |(item, _)| *item)
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rgb(r, g, b) => write!(f, "#{r:02X}{g:02X}{b:02X}"),
            named @ (Self::Red
            | Self::Yellow
            | Self::Blue
            | Self::Green
            | Self::Grey
            | Self::White
            | Self::Accent) => {
                let name = Self::NAMES
                    .iter()
                    .find(|(_, c)| c == named)
                    .map_or("grey", |(n, _)| n);
                f.write_str(name)
            }
        }
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Text(String),
            Rgb([u8; 3]),
        }
        match Raw::deserialize(d)? {
            Raw::Text(s) => Self::parse(&s).ok_or_else(|| {
                de::Error::custom(format!(
                    "unknown colour {s:?}: use red, yellow, blue, green, grey, white, accent, #RRGGBB or [r,g,b]"
                ))
            }),
            Raw::Rgb([r, g, b]) => Ok(Self::Rgb(r, g, b)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(json: &str) -> Result<Color, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn colours_in_every_form() {
        assert_eq!(c(r#""Red""#).unwrap(), Color::Red);
        assert_eq!(c(r##""#FF0000""##).unwrap(), Color::Rgb(255, 0, 0));
        assert_eq!(c(r#""00ff00""#).unwrap(), Color::Rgb(0, 255, 0));
        assert_eq!(c(r##""#f80""##).unwrap(), Color::Rgb(255, 136, 0));
        assert_eq!(c("[1,2,3]").unwrap(), Color::Rgb(1, 2, 3));
        c(r#""purpleish""#).unwrap_err();
        c("[1,2,300]").unwrap_err();
        assert_eq!(Color::Rgb(255, 0, 0).to_string(), "#FF0000");
        assert_eq!(Color::Accent.to_string(), "accent");
    }

    #[test]
    fn any_colour_maps_to_a_feed_colour() {
        assert_eq!(Color::Rgb(255, 0, 0).item(), ItemColor::Red);
        assert_eq!(Color::Rgb(255, 165, 0).item(), ItemColor::Yellow);
        assert_eq!(Color::Rgb(0, 0, 255).item(), ItemColor::Blue);
        assert_eq!(Color::Rgb(0, 200, 0).item(), ItemColor::Green);
        assert_eq!(Color::Rgb(128, 128, 128).item(), ItemColor::Grey);
    }
}
