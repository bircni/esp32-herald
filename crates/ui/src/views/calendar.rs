//! Today in large print, the month at a glance, and daylight.

use embedded_graphics::{prelude::*, primitives::Rectangle};

use super::{region, Region};
use crate::{
    app::App,
    gfx::{self, Align, Canvas, Font},
    model::{days_in_month, weekday_of, Env, LocalTime},
    theme::Theme,
};

const AREA: Rectangle = gfx::rect(0, 0, 480, 304);
const LEFT_X: i32 = 104;
const GRID_X: i32 = 228;
const COL_W: i32 = 35;
const HEADER_Y: i32 = 26;
const GRID_Y: i32 = 58;
const ROW_H: i32 = 40;

fn daylight(app: &App) -> Option<((u8, u8), (u8, u8))> {
    let w = app.weather.as_ref()?;
    Some((w.sunrise?, w.sunset?))
}

pub(crate) fn regions(app: &App, env: &Env) -> Vec<Region> {
    let date = env.local.map_or(-1, |t| {
        i64::from(t.year) << 16 | i64::from(t.month) << 8 | i64::from(t.day)
    });
    let light = daylight(app).map_or(-1, |((a, b), (c, d))| {
        i64::from(a) << 24 | i64::from(b) << 16 | i64::from(c) << 8 | i64::from(d)
    });
    vec![region(AREA, &[date, light])]
}

pub(crate) fn paint(app: &App, c: &mut Canvas<'_>, env: &Env, theme: &Theme) {
    let Some(t) = env.local else {
        gfx::text(
            c,
            Font::Medium,
            "Waiting for the time",
            Point::new(240, 150),
            Align::Center,
            theme.dim,
        );
        return;
    };
    let left = |c: &mut Canvas<'_>, font, text: &str, y, color| {
        gfx::text(c, font, text, Point::new(LEFT_X, y), Align::Center, color);
    };
    left(c, Font::Bold, t.weekday_name(), 34, theme.accent);
    left(c, Font::Huge, &t.day.to_string(), 92, theme.fg);
    left(
        c,
        Font::Medium,
        &format!("{} {}", t.month_name(), t.year),
        146,
        theme.fg,
    );
    left(
        c,
        Font::Small,
        &format!("Week {}  \u{b7}  day {}", t.iso_week(), t.day_of_year()),
        174,
        theme.dim,
    );
    if let Some(((rh, rm), (sh, sm))) = daylight(app) {
        let minutes = (i32::from(sh) * 60 + i32::from(sm)) - (i32::from(rh) * 60 + i32::from(rm));
        left(
            c,
            Font::Small,
            &format!("Sunrise  {rh:02}:{rm:02}"),
            222,
            theme.fg,
        );
        left(
            c,
            Font::Small,
            &format!("Sunset  {sh:02}:{sm:02}"),
            246,
            theme.fg,
        );
        if minutes > 0 {
            left(
                c,
                Font::Small,
                &format!("{}h {:02}m of daylight", minutes / 60, minutes % 60),
                270,
                theme.dim,
            );
        }
    }
    paint_month(c, &t, theme);
}

fn paint_month(c: &mut Canvas<'_>, t: &LocalTime, theme: &Theme) {
    let col = |i: i32| GRID_X + COL_W * i + COL_W / 2;
    for (i, name) in ["M", "T", "W", "T", "F", "S", "S"].into_iter().enumerate() {
        gfx::text(
            c,
            Font::Small,
            name,
            Point::new(col(i as i32), HEADER_Y),
            Align::Center,
            theme.dim,
        );
    }
    let first = i32::from(weekday_of(t.year, t.month, 1));
    for day in 1..=i32::from(days_in_month(t.year, t.month)) {
        let cell = first + day - 1;
        let (column, row) = (cell % 7, cell / 7);
        let at = Point::new(col(column), GRID_Y + ROW_H * row);
        let color = if day == i32::from(t.day) {
            gfx::circle(c, at, 32, theme.accent);
            theme.on_accent
        } else if column >= 5 {
            theme.dim
        } else {
            theme.fg
        };
        gfx::text(c, Font::Medium, &day.to_string(), at, Align::Center, color);
    }
}
