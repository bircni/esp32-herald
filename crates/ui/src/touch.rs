//! Touch input: turns raw samples from a touch panel into [`Action`]s.
//!
//! Owns what only touch boards need: the calibration (and the calibration
//! screen's progress), the gesture recogniser, and swallowing the touch that
//! wakes a dark screen. Boards with buttons skip all of this and hand the
//! app actions directly.

use embedded_graphics::prelude::*;

use crate::{
    action::Action,
    gesture::{Dir, Gesture, Recognizer},
    model::{Calibration, RawTouch},
    views::{self, setup::CalibrateState},
};

pub struct Touch {
    calibration: Option<Calibration>,
    /// The calibration screen's progress while it is shown.
    pub(crate) calibrate: Option<CalibrateState>,
    gesture: Recognizer,
    /// The touch that woke the display must not trigger anything else.
    swallow: bool,
}

/// What one touch sample led to.
pub(crate) enum Fed {
    /// Nothing (yet).
    Nothing,
    /// The finger is down, or it lifted without a gesture.
    Contact,
    /// A gesture, as an action for the app.
    Action(Action),
    /// The calibration screen is done.
    Calibrated(Calibration),
}

impl Touch {
    /// `calibration` is `None` on first boot, which starts with the
    /// calibration screen; so does one made for another screen size.
    pub(crate) fn new(calibration: Option<Calibration>) -> Self {
        let calibration = calibration.filter(|c| c.fits(views::W, views::H));
        Self {
            calibrate: calibration.is_none().then(CalibrateState::default),
            calibration,
            gesture: Recognizer::new(),
            swallow: false,
        }
    }

    pub(crate) const fn calibrating(&self) -> bool {
        self.calibrate.is_some()
    }

    pub(crate) fn start_calibration(&mut self) {
        self.calibrate = Some(CalibrateState::default());
    }

    /// The finger is on the panel (rotation waits until it lifts).
    pub(crate) const fn is_pressed(&self) -> bool {
        self.gesture.is_pressed()
    }

    /// Ignores the current touch until the finger lifts, e.g. the one that
    /// woke the screen.
    pub(crate) const fn swallow(&mut self) {
        self.swallow = true;
    }

    /// Feeds one sample. `upside_down` turns touch positions like the
    /// picture.
    pub(crate) fn feed(&mut self, raw: Option<RawTouch>, upside_down: bool, now_ms: u64) -> Fed {
        if let Some(state) = self.calibrate.as_mut() {
            return match state.feed(raw) {
                Some(calibration) => {
                    self.calibration = Some(calibration);
                    self.calibrate = None;
                    self.gesture.reset();
                    self.swallow = true;
                    Fed::Calibrated(calibration)
                }
                None => Fed::Nothing,
            };
        }
        let point = match (raw, self.calibration.as_ref()) {
            (Some(raw), Some(cal)) => {
                let p = cal.map(raw);
                Some(if upside_down {
                    Point::new(views::W - 1 - p.x, views::H - 1 - p.y)
                } else {
                    p
                })
            }
            _ => None,
        };
        if self.swallow {
            self.gesture.reset();
            if point.is_none() {
                self.swallow = false;
            }
            return if point.is_some() {
                Fed::Contact
            } else {
                Fed::Nothing
            };
        }
        match self.gesture.update(point, now_ms) {
            Some(g) => gesture_action(g).map_or(Fed::Contact, Fed::Action),
            None if point.is_some() => Fed::Contact,
            None => Fed::Nothing,
        }
    }
}

/// The action a gesture stands for: swiping left shows the next view.
#[must_use]
pub const fn gesture_action(gesture: Gesture) -> Option<Action> {
    match gesture {
        Gesture::Tap(p) => Some(Action::Tap(p)),
        Gesture::LongPress(p) => Some(Action::LongPress(p)),
        Gesture::Swipe(Dir::Left) => Some(Action::Next),
        Gesture::Swipe(Dir::Right) => Some(Action::Previous),
        Gesture::Swipe(Dir::Up | Dir::Down) => None,
    }
}
