//! First-boot screens: touch calibration and the `WiFi` setup instructions.

use embedded_graphics::prelude::*;

use super::{Region, region};
use crate::{
    app::App,
    gfx::{self, Align, Canvas, Font},
    model::{Calibration, Env, NetState, RawTouch, screen},
    theme::Theme,
};

/// Calibration crosses: top-left, top-right, bottom-right, bottom-left.
pub const TARGETS: [Point; 4] = [
    Point::new(30, 30),
    Point::new(450, 30),
    Point::new(450, 290),
    Point::new(30, 290),
];

/// Samples at the start of a press are unreliable on resistive panels.
const SKIP_SAMPLES: u32 = 2;
const MIN_SAMPLES: u32 = 3;

#[derive(Default)]
pub struct CalibrateState {
    step: usize,
    raw: [RawTouch; 4],
    sum: (u32, u32),
    count: u32,
    seen: u32,
    missing: u8,
    failed: bool,
}

impl CalibrateState {
    /// Feed one raw sample per tick. Returns the calibration once all four
    /// crosses have been tapped.
    pub fn feed(&mut self, raw: Option<RawTouch>) -> Option<Calibration> {
        match raw {
            Some(r) => {
                self.missing = 0;
                self.seen += 1;
                if self.seen > SKIP_SAMPLES {
                    self.sum.0 += u32::from(r.x);
                    self.sum.1 += u32::from(r.y);
                    self.count += 1;
                }
                None
            }
            None if self.seen == 0 => None,
            None => {
                self.missing += 1;
                if self.missing < 2 {
                    return None;
                }
                let (sum, count) = (self.sum, self.count);
                self.sum = (0, 0);
                self.count = 0;
                self.seen = 0;
                self.missing = 0;
                if count < MIN_SAMPLES {
                    return None; // too short, tap again
                }
                self.raw[self.step] = RawTouch {
                    x: (sum.0 / count) as u16,
                    y: (sum.1 / count) as u16,
                };
                self.step += 1;
                if self.step < TARGETS.len() {
                    return None;
                }
                let result = Calibration::from_corners(&TARGETS, &self.raw);
                self.step = 0;
                self.failed = result.is_none();
                result
            }
        }
    }
}

pub(crate) fn calibrate_regions(app: &App) -> Vec<Region> {
    let (step, failed) = app
        .calibrate
        .as_ref()
        .map_or((0, false), |s| (s.step as i64, s.failed));
    vec![region(screen(), &[step, i64::from(failed)])]
}

pub(crate) fn paint_calibrate(app: &App, c: &mut Canvas<'_>, theme: &Theme) {
    let Some(state) = app.calibrate.as_ref() else {
        return;
    };
    let center = |c: &mut Canvas<'_>, font, text: &str, y, color| {
        gfx::text(c, font, text, Point::new(240, y), Align::Center, color);
    };
    center(c, Font::Bold, "Touch calibration", 110, theme.fg);
    center(
        c,
        Font::Small,
        "Press and hold the centre of the cross",
        145,
        theme.dim,
    );
    center(
        c,
        Font::Medium,
        &format!("{} / 4", state.step + 1),
        185,
        theme.fg,
    );
    if state.failed {
        center(
            c,
            Font::Small,
            "That did not work, please try again",
            225,
            theme.warn,
        );
    }
    let t = TARGETS[state.step.min(TARGETS.len() - 1)];
    gfx::line(
        c,
        t - Point::new(16, 0),
        t + Point::new(16, 0),
        3,
        theme.accent,
    );
    gfx::line(
        c,
        t - Point::new(0, 16),
        t + Point::new(0, 16),
        3,
        theme.accent,
    );
}

pub(crate) fn wifi_regions(env: &Env) -> Vec<Region> {
    let key = match &env.net {
        NetState::Setup { ssid, password, ip } => {
            gfx::key(&[gfx::str_key(ssid), gfx::str_key(password), gfx::str_key(ip)]) as i64
        }
        NetState::Offline | NetState::Connecting { .. } | NetState::Connected { .. } => 0,
    };
    vec![region(screen(), &[key])]
}

pub(crate) fn paint_wifi(c: &mut Canvas<'_>, env: &Env, theme: &Theme) {
    let NetState::Setup { ssid, password, ip } = &env.net else {
        return;
    };
    let center = |c: &mut Canvas<'_>, font, text: &str, y, color| {
        gfx::text(c, font, text, Point::new(240, y), Align::Center, color);
    };
    center(c, Font::Large, "WiFi setup", 30, theme.fg);
    center(c, Font::Small, "1. Join this WiFi network", 72, theme.dim);
    center(c, Font::Bold, ssid, 96, theme.accent);
    center(c, Font::Small, "Password", 126, theme.dim);
    center(c, Font::Bold, password, 148, theme.accent);
    center(c, Font::Small, "2. Open in your browser", 182, theme.dim);
    center(c, Font::Bold, &format!("http://{ip}"), 206, theme.fg);
    center(
        c,
        Font::Small,
        "3. Enter your home WiFi (2.4 GHz)",
        240,
        theme.dim,
    );
    center(
        c,
        Font::Small,
        "Tap to continue without WiFi",
        296,
        theme.dim,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(state: &mut CalibrateState, raw: RawTouch, samples: usize) -> Option<Calibration> {
        for _ in 0..samples {
            assert_eq!(state.feed(Some(raw)), None);
        }
        state.feed(None);
        state.feed(None)
    }

    fn raw_for(p: Point) -> RawTouch {
        RawTouch {
            x: (p.x * 11 + 300) as u16,
            y: (p.y * 7 + 250) as u16,
        }
    }

    #[test]
    fn four_presses_produce_a_calibration() {
        let mut s = CalibrateState::default();
        for t in &TARGETS[..3] {
            assert_eq!(press(&mut s, raw_for(*t), 8), None);
        }
        let cal = press(&mut s, raw_for(TARGETS[3]), 8).expect("calibration");
        assert_eq!(cal.map(raw_for(Point::new(240, 160))), Point::new(240, 160));
    }

    #[test]
    fn short_presses_are_ignored() {
        let mut s = CalibrateState::default();
        assert_eq!(press(&mut s, raw_for(TARGETS[0]), 3), None);
        assert_eq!(s.step, 0);
    }

    #[test]
    fn noisy_first_samples_are_skipped() {
        let mut s = CalibrateState::default();
        s.feed(Some(RawTouch { x: 4000, y: 10 }));
        s.feed(Some(RawTouch { x: 10, y: 4000 }));
        press(&mut s, raw_for(TARGETS[0]), 6);
        assert_eq!(s.raw[0], raw_for(TARGETS[0]));
    }

    #[test]
    fn bogus_taps_restart_the_sequence() {
        let mut s = CalibrateState::default();
        for _ in 0..4 {
            assert_eq!(press(&mut s, RawTouch { x: 2000, y: 2000 }, 8), None);
        }
        assert_eq!(s.step, 0);
        assert!(s.failed);
    }
}
