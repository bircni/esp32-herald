//! The rotary switch (up, down, press) and the MENU and EXIT buttons. All
//! pull to ground when pressed.

use esp_idf_svc::hal::gpio::{Input, PinDriver};
use ui::epaper::Button;

/// A press must read the same for this many polls in a row.
const STABLE_POLLS: u8 = 2;

struct Key<'d> {
    pin: PinDriver<'d, Input>,
    button: Button,
    pressed: bool,
    count: u8,
}

pub struct Buttons<'d> {
    keys: Vec<Key<'d>>,
}

impl<'d> Buttons<'d> {
    pub fn new(pins: impl IntoIterator<Item = (PinDriver<'d, Input>, Button)>) -> Self {
        Self {
            keys: pins
                .into_iter()
                .map(|(pin, button)| Key {
                    pin,
                    button,
                    pressed: false,
                    count: 0,
                })
                .collect(),
        }
    }

    /// Buttons pressed since the last poll. Call every 10-30 ms.
    pub fn poll(&mut self) -> Vec<Button> {
        let mut pressed = Vec::new();
        for key in &mut self.keys {
            let down = key.pin.is_low();
            if down == key.pressed {
                key.count = 0;
                continue;
            }
            key.count += 1;
            if key.count >= STABLE_POLLS {
                key.pressed = down;
                key.count = 0;
                if down {
                    pressed.push(key.button);
                }
            }
        }
        pressed
    }
}
