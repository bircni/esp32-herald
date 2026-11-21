//! The next hours as a temperature curve with rain chances, and the next
//! days as cards.

use embedded_graphics::{prelude::*, primitives::Rectangle};

use super::{icons, region, Region};
use crate::{
    app::App,
    gfx::{self, Align, Canvas, Font},
    model::weekday_abbr,
    theme::Theme,
    weather::{deg, Weather},
};

const AREA: Rectangle = gfx::rect(0, 0, 480, 304);
const COL_W: i32 = 38;
const CURVE_TOP: i32 = 62;
const CURVE_BOTTOM: i32 = 116;
const RAIN_BASE: i32 = 160;
const RAIN_H: i32 = 30;
const HOUR_Y: i32 = 174;
const CARD_Y: i32 = 192;
const CARD_W: u32 = 108;
const CARD_H: u32 = 106;

pub(crate) fn regions(app: &App) -> Vec<Region> {
    let mut key = vec![gfx::str_key(&app.settings.location)];
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

fn col_x(i: usize, n: usize) -> i32 {
    240 - COL_W * (n as i32 - 1) / 2 + COL_W * i as i32
}

pub(crate) fn paint(app: &App, c: &mut Canvas<'_>, theme: &Theme) {
    let center = |c: &mut Canvas<'_>, font, text: &str, y, color| {
        gfx::text(c, font, text, Point::new(240, y), Align::Center, color);
    };
    if !app.settings.has_location() {
        center(c, Font::Bold, "No weather location", 140, theme.fg);
        center(
            c,
            Font::Small,
            "Set one on the device's web page",
            172,
            theme.dim,
        );
        return;
    }
    let Some(w) = app.weather.as_ref().filter(|w| !w.hours.is_empty()) else {
        center(c, Font::Medium, "Loading forecast...", 150, theme.dim);
        return;
    };
    gfx::text(
        c,
        Font::Small,
        "Next hours",
        Point::new(16, 18),
        Align::Left,
        theme.dim,
    );
    gfx::text(
        c,
        Font::Small,
        &app.settings.location,
        Point::new(464, 18),
        Align::Right,
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
    // At least 4 degrees of range, so a flat day stays flat on screen.
    let span = (hi - lo).max(4.0);
    let mid = (hi + lo) / 2.0;
    let y_of = |t: f32| {
        let f = (t - (mid - span / 2.0)) / span;
        CURVE_BOTTOM - (f * (CURVE_BOTTOM - CURVE_TOP) as f32).round() as i32
    };
    let points: Vec<Point> = w
        .hours
        .iter()
        .enumerate()
        .map(|(i, h)| Point::new(col_x(i, n), y_of(h.temp_c)))
        .collect();

    for (i, h) in w.hours.iter().enumerate() {
        let x = col_x(i, n);
        if h.rain_pct > 0 {
            let bar_h = (i32::from(h.rain_pct) * RAIN_H / 100).max(2);
            gfx::round_rect(
                c,
                gfx::rect(x - 10, RAIN_BASE - bar_h, 20, bar_h as u32),
                3,
                theme.info,
            );
        }
        gfx::text(
            c,
            Font::Small,
            &format!("{:02}", h.hour),
            Point::new(x, HOUR_Y),
            Align::Center,
            theme.dim,
        );
    }
    gfx::fill(c, gfx::rect(12, RAIN_BASE, 456, 1), theme.panel);

    for pair in points.windows(2) {
        gfx::line(c, pair[0], pair[1], 3, theme.accent);
    }
    for (i, (p, h)) in points.iter().zip(&w.hours).enumerate() {
        gfx::circle(c, *p, 9, theme.accent);
        if i % 2 == 0 {
            gfx::text(
                c,
                Font::Small,
                &deg(h.temp_c),
                *p - Point::new(0, 18),
                Align::Center,
                theme.fg,
            );
        }
    }
}

fn paint_days(c: &mut Canvas<'_>, w: &Weather, theme: &Theme) {
    let n = w.days.len() as i32;
    let gap = 8;
    let total = n * CARD_W as i32 + (n - 1).max(0) * gap;
    for (i, d) in w.days.iter().enumerate() {
        let x = 240 - total / 2 + i as i32 * (CARD_W as i32 + gap);
        let card = gfx::rect(x, CARD_Y, CARD_W, CARD_H);
        gfx::round_rect(c, card, 12, theme.panel);
        let cx = card.center().x;
        let name = if i == 0 {
            "Today"
        } else {
            weekday_abbr(d.weekday)
        };
        gfx::text(
            c,
            Font::Bold,
            name,
            Point::new(cx, CARD_Y + 16),
            Align::Center,
            theme.fg,
        );
        icons::weather(
            c,
            Point::new(cx, CARD_Y + 52),
            0.3,
            d.condition,
            true,
            theme,
            theme.panel,
        );
        let range = format!("{} / {}", deg(d.max_c), deg(d.min_c));
        gfx::text(
            c,
            Font::Small,
            &range,
            Point::new(cx, CARD_Y + 90),
            Align::Center,
            theme.fg,
        );
    }
}
