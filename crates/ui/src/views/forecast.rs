//! The next hours as a temperature curve with rain chances, and the next
//! days as cards.

use embedded_graphics::{prelude::*, primitives::Rectangle};

use super::{Region, icons, region};
use crate::{
    app::App,
    gfx::{self, Align, Canvas, Font},
    model::weekday_abbr,
    theme::Theme,
    weather::{Weather, deg},
};

const AREA: Rectangle = gfx::rect(0, 0, 480, 304);
// The same layout as the web app's mirror (`drawForecast`).
const COL_W: i32 = 36;
/// The curve spans 60 px above this line.
const CURVE_BOTTOM: i32 = 110;
const CURVE_H: f32 = 60.0;
const RAIN_BASE: i32 = 168;
const HOUR_Y: i32 = 180;
const CARD_Y: i32 = 194;
const CARD_SIZE: u32 = 104;
const CARD_STEP: i32 = 112;

pub(crate) fn regions(app: &App) -> Vec<Region> {
    let mut key = vec![app.weather.is_some().into()];
    if let Some(w) = &app.weather {
        for h in &w.hours {
            key.extend([
                i64::from(h.hour),
                h.temp_c.round() as i64,
                i64::from(h.rain_pct),
                h.condition as i64,
            ]);
        }
        for d in &w.days {
            key.extend([
                i64::from(d.weekday),
                d.condition as i64,
                d.min_c.round() as i64,
                d.max_c.round() as i64,
            ]);
        }
    }
    vec![region(AREA, &key)]
}

const fn col_x(i: usize, n: usize) -> i32 {
    240 - COL_W * (n as i32 - 1) / 2 + COL_W * i as i32
}

pub(crate) fn paint(app: &App, c: &mut Canvas<'_>, theme: &Theme) {
    let Some(w) = app.weather.as_ref().filter(|w| !w.hours.is_empty()) else {
        let center = |c: &mut Canvas<'_>, font, text: &str, y, color| {
            gfx::text(c, font, text, Point::new(240, y), Align::Center, color);
        };
        center(c, Font::Large, "No forecast yet", 140, theme.fg);
        center(c, Font::Body, "Send one over MQTT", 172, theme.dim);
        return;
    };
    gfx::text(
        c,
        Font::Body,
        "Next hours",
        Point::new(16, 18),
        Align::Left,
        theme.dim,
    );
    paint_hours(c, w, theme);
    paint_days(c, w, theme);
}

fn paint_hours(c: &mut Canvas<'_>, w: &Weather, theme: &Theme) {
    let n = w.hours.len();
    let (lo, hi) = w.hours.iter().fold((f32::MAX, f32::MIN), |(lo, hi), h| {
        (lo.min(h.temp_c), hi.max(h.temp_c))
    });
    let span = (hi - lo).max(1.0);
    let y_of = |t: f32| CURVE_BOTTOM - ((t - lo) / span * CURVE_H).round() as i32;
    let points: Vec<Point> = w
        .hours
        .iter()
        .enumerate()
        .map(|(i, h)| Point::new(col_x(i, n), y_of(h.temp_c)))
        .collect();

    for (i, h) in w.hours.iter().enumerate() {
        let x = col_x(i, n);
        let bar_h = (f32::from(h.rain_pct) * 0.3).round().max(1.0) as i32;
        gfx::round_rect(
            c,
            gfx::rect(x - 8, RAIN_BASE - bar_h, 16, bar_h as u32),
            2,
            theme.info,
        );
        gfx::text(
            c,
            Font::Small,
            &format!("{:02}", h.hour),
            Point::new(x, HOUR_Y),
            Align::Center,
            theme.dim,
        );
    }
    for pair in points.windows(2) {
        gfx::line(c, pair[0], pair[1], 3, theme.accent);
    }
    for (i, (p, h)) in points.iter().zip(&w.hours).enumerate() {
        gfx::circle(c, *p, 7, theme.accent);
        if i % 2 == 0 {
            gfx::text(
                c,
                Font::Body,
                &deg(h.temp_c),
                *p - Point::new(0, 16),
                Align::Center,
                theme.fg,
            );
        }
    }
}

fn paint_days(c: &mut Canvas<'_>, w: &Weather, theme: &Theme) {
    for (i, d) in w.days.iter().take(4).enumerate() {
        let x = 24 + CARD_STEP * i as i32;
        gfx::round_rect(
            c,
            gfx::rect(x, CARD_Y, CARD_SIZE, CARD_SIZE),
            8,
            theme.panel,
        );
        let cx = x + CARD_SIZE as i32 / 2;
        let name = if i == 0 {
            "Today"
        } else {
            weekday_abbr(d.weekday)
        };
        gfx::text(
            c,
            Font::Medium,
            name,
            Point::new(cx, CARD_Y + 18),
            Align::Center,
            theme.fg,
        );
        icons::weather(
            c,
            Point::new(cx, CARD_Y + 50),
            32,
            d.condition,
            true,
            1.7,
            theme.dim,
        );
        gfx::text(
            c,
            Font::Body,
            &format!("{} / {}", deg(d.max_c), deg(d.min_c)),
            Point::new(cx, CARD_Y + 86),
            Align::Center,
            theme.fg,
        );
    }
}
