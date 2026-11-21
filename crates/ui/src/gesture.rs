//! Turns a stream of touch samples into taps, long presses and swipes.
//!
//! Written for a resistive panel: samples are noisy, the last sample before
//! lift-off is often wrong, and contact can drop out for a single reading.

use embedded_graphics::prelude::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    Tap(Point),
    LongPress(Point),
    Swipe(Dir),
}

/// Movement below this many pixels still counts as "did not move".
const TAP_SLOP: i32 = 22;
/// Minimum travel for a swipe.
const SWIPE_MIN: i32 = 45;
const LONG_PRESS_MS: u64 = 700;
/// Consecutive empty samples needed before a release is accepted.
const RELEASE_SAMPLES: u8 = 2;

struct Press {
    start: Point,
    start_ms: u64,
    last: Point,
    /// The sample before `last`; more trustworthy than `last` at lift-off.
    prev: Option<Point>,
    max_travel: i32,
    long_fired: bool,
    missing: u8,
}

#[derive(Default)]
pub struct Recognizer {
    press: Option<Press>,
}

fn travel(a: Point, b: Point) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

impl Recognizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets the current press without emitting anything.
    pub fn reset(&mut self) {
        self.press = None;
    }

    pub fn is_pressed(&self) -> bool {
        self.press.is_some()
    }

    /// Feed one sample per tick. `touch` is `None` while nothing is pressed.
    pub fn update(&mut self, touch: Option<Point>, now_ms: u64) -> Option<Gesture> {
        match (touch, self.press.as_mut()) {
            (Some(p), None) => {
                self.press = Some(Press {
                    start: p,
                    start_ms: now_ms,
                    last: p,
                    prev: None,
                    max_travel: 0,
                    long_fired: false,
                    missing: 0,
                });
                None
            }
            (Some(p), Some(press)) => {
                press.missing = 0;
                press.prev = Some(press.last);
                press.last = p;
                press.max_travel = press.max_travel.max(travel(press.start, p));
                if !press.long_fired
                    && press.max_travel < TAP_SLOP
                    && now_ms.saturating_sub(press.start_ms) >= LONG_PRESS_MS
                {
                    press.long_fired = true;
                    return Some(Gesture::LongPress(press.start));
                }
                None
            }
            (None, Some(press)) => {
                press.missing += 1;
                if press.missing < RELEASE_SAMPLES {
                    return None;
                }
                let press = self.press.take()?;
                if press.long_fired {
                    return None;
                }
                let end = press.prev.unwrap_or(press.last);
                let (dx, dy) = (end.x - press.start.x, end.y - press.start.y);
                if dx.abs().max(dy.abs()) >= SWIPE_MIN {
                    Some(Gesture::Swipe(if dx.abs() > dy.abs() {
                        if dx > 0 {
                            Dir::Right
                        } else {
                            Dir::Left
                        }
                    } else if dy > 0 {
                        Dir::Down
                    } else {
                        Dir::Up
                    }))
                } else if press.max_travel < TAP_SLOP {
                    Some(Gesture::Tap(press.start))
                } else {
                    None
                }
            }
            (None, None) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs samples at 30 ms intervals followed by a release and returns
    /// every gesture produced.
    fn run(samples: &[Option<(i32, i32)>]) -> Vec<Gesture> {
        let mut r = Recognizer::new();
        let mut out = Vec::new();
        let mut t = 0;
        for s in samples.iter().copied().chain([None, None, None]) {
            out.extend(r.update(s.map(|(x, y)| Point::new(x, y)), t));
            t += 30;
        }
        out
    }

    #[test]
    fn tap() {
        let g = run(&[Some((100, 100)), Some((102, 99)), Some((101, 101))]);
        assert_eq!(g, vec![Gesture::Tap(Point::new(100, 100))]);
    }

    #[test]
    fn single_sample_tap() {
        assert_eq!(run(&[Some((5, 5))]), vec![Gesture::Tap(Point::new(5, 5))]);
    }

    #[test]
    fn swipes_in_all_directions() {
        let path = |dx: i32, dy: i32| -> Vec<Option<(i32, i32)>> {
            (0..=6)
                .map(|i| Some((160 + dx * i, 240 + dy * i)))
                .collect()
        };
        assert_eq!(run(&path(0, -20)), vec![Gesture::Swipe(Dir::Up)]);
        assert_eq!(run(&path(0, 20)), vec![Gesture::Swipe(Dir::Down)]);
        assert_eq!(run(&path(-20, 3)), vec![Gesture::Swipe(Dir::Left)]);
        assert_eq!(run(&path(20, -3)), vec![Gesture::Swipe(Dir::Right)]);
    }

    #[test]
    fn lift_off_glitch_does_not_turn_a_tap_into_a_swipe() {
        // The final sample jumps far away, as resistive panels do on release.
        let g = run(&[
            Some((100, 100)),
            Some((101, 100)),
            Some((100, 101)),
            Some((101, 99)),
        ]);
        assert_eq!(g, vec![Gesture::Tap(Point::new(100, 100))]);
        let g = run(&[Some((100, 100)), Some((101, 100)), Some((250, 400))]);
        assert_eq!(
            g,
            vec![],
            "moved too far for a tap, glitch ignored for swipe"
        );
    }

    #[test]
    fn one_missing_sample_does_not_end_the_press() {
        let g = run(&[
            Some((160, 400)),
            Some((160, 370)),
            None,
            Some((160, 330)),
            Some((160, 300)),
            Some((160, 280)),
        ]);
        assert_eq!(g, vec![Gesture::Swipe(Dir::Up)]);
    }

    #[test]
    fn long_press_fires_once_and_suppresses_tap() {
        let held: Vec<_> = (0..40).map(|_| Some((50, 60))).collect();
        assert_eq!(run(&held), vec![Gesture::LongPress(Point::new(50, 60))]);
    }

    #[test]
    fn moving_finger_is_not_a_long_press() {
        let mut samples: Vec<_> = (0..40).map(|i| Some((50 + i * 3, 60))).collect();
        samples.push(Some((170, 60)));
        assert_eq!(run(&samples), vec![Gesture::Swipe(Dir::Right)]);
    }

    #[test]
    fn reset_drops_the_press() {
        let mut r = Recognizer::new();
        r.update(Some(Point::new(1, 1)), 0);
        assert!(r.is_pressed());
        r.reset();
        assert_eq!(r.update(None, 30), None);
        assert_eq!(r.update(None, 60), None);
    }
}
