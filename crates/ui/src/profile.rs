//! What a board can do: its screen and which inputs and outputs it has.
//! The app adapts to it, and `/api/state` reports it so the web app shows
//! only the controls that do something on this board.

use embedded_graphics::{prelude::*, primitives::Rectangle};
use serde::Serialize;

use crate::model::Orientation;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent capabilities, not a state machine"
)]
pub struct Profile {
    /// Short name, as in `firmware/boards/<name>`.
    pub name: &'static str,
    /// Upright size in pixels, as the UI draws it.
    pub width: i32,
    pub height: i32,
    /// Colour display; otherwise black and white.
    pub color: bool,
    /// A touch panel (resistive ones need calibrating).
    pub touch: bool,
    /// Physical buttons.
    pub buttons: bool,
    /// Dimmable backlight: brightness, sleep and night mode apply.
    pub backlight: bool,
    pub led: bool,
    pub speaker: bool,
    /// Keeps the whole picture, so `/api/frame.bmp` can send it.
    pub frame: bool,
    /// The screen can be used upright or in landscape (the `orientation`
    /// setting); `width` and `height` are its default way.
    pub rotatable: bool,
}

impl Profile {
    /// Freenove ESP32 Display 3.5" (FNK0103): 480x320 colour TFT, resistive
    /// touch, backlight, RGB LED, speaker.
    pub const FREENOVE_35: Self = Self {
        name: "freenove-35",
        width: 480,
        height: 320,
        color: true,
        touch: true,
        buttons: false,
        backlight: true,
        led: true,
        speaker: true,
        frame: false,
        rotatable: false,
    };

    /// Elecrow `CrowPanel` 5.79" e-paper, upright: 272x792 black and white,
    /// a rotary switch and two buttons.
    pub const CROWPANEL_579: Self = Self {
        name: "crowpanel-579",
        width: 272,
        height: 792,
        color: false,
        touch: false,
        buttons: true,
        backlight: false,
        led: false,
        speaker: false,
        frame: true,
        rotatable: true,
    };

    /// Every known board.
    pub const ALL: [Self; 2] = [Self::FREENOVE_35, Self::CROWPANEL_579];

    #[must_use]
    pub fn by_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name == name)
    }

    /// The whole screen, in its default orientation.
    #[must_use]
    pub const fn area(&self) -> Rectangle {
        Rectangle::new(
            Point::zero(),
            Size::new(self.width as u32, self.height as u32),
        )
    }

    /// The screen as used with `orientation`: width and height swap when a
    /// rotatable screen is turned the other way.
    #[must_use]
    pub const fn oriented(self, orientation: Orientation) -> Self {
        let (short, long) = if self.width < self.height {
            (self.width, self.height)
        } else {
            (self.height, self.width)
        };
        let (width, height) = match orientation {
            _ if !self.rotatable => (self.width, self.height),
            Orientation::Default => (self.width, self.height),
            Orientation::Upright => (short, long),
            Orientation::Landscape => (long, short),
        };
        Self {
            width,
            height,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boards_are_found_by_name() {
        for p in Profile::ALL {
            assert_eq!(Profile::by_name(p.name), Some(p));
        }
        assert_eq!(Profile::by_name("nope"), None);
    }

    #[test]
    fn rotatable_screens_turn() {
        let paper = Profile::CROWPANEL_579;
        let wide = paper.oriented(Orientation::Landscape);
        assert_eq!((wide.width, wide.height), (792, 272));
        assert_eq!(wide.oriented(Orientation::Upright).width, 272);
        assert_eq!(paper.oriented(Orientation::Default), paper);
        let tft = Profile::FREENOVE_35;
        assert_eq!(tft.oriented(Orientation::Upright), tft, "not rotatable");
    }
}
