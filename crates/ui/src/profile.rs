//! What a board can do: its screen and which inputs and outputs it has.
//! The app adapts to it, and `/api/state` reports it so the web app shows
//! only the controls that do something on this board.

use embedded_graphics::{prelude::*, primitives::Rectangle};
use serde::Serialize;

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
    };

    /// Every known board.
    pub const ALL: [Self; 2] = [Self::FREENOVE_35, Self::CROWPANEL_579];

    #[must_use]
    pub fn by_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name == name)
    }

    /// The whole screen.
    #[must_use]
    pub const fn area(&self) -> Rectangle {
        Rectangle::new(
            Point::zero(),
            Size::new(self.width as u32, self.height as u32),
        )
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
}
