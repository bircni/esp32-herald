//! Today in large print, the month at a glance, and daylight.

use embedded_graphics::{prelude::*, primitives::Rectangle};

use super::{
    Region,
    icons::{self, Symbol},
    region,
};
use crate::{
    app::App,
    gfx::{self, Align, Canvas, Font},
    i18n::{Language, Text},
    model::{Env, LocalTime, days_in_month, weekday_of},
    theme::Theme,
};

const AREA: Rectangle = gfx::rect(0, 0, 480, 304);
// The same layout as the calendar on the web app's mirror.
const LEFT_X: i32 = 90;
/// Centre of the sunrise and sunset icons.
const SUN_X: i32 = 41;
/// Centre of the first column of the month grid.
const GRID_X: i32 = 214;
const COL_W: i32 = 36;
const HEADER_Y: i32 = 40;
const GRID_Y: i32 = 72;
const ROW_H: i32 = 38;

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
            app.lang().text(Text::WaitingForTime),
            Point::new(240, 150),
            Align::Center,
            theme.dim,
        );
        return;
    };
    let left = |c: &mut Canvas<'_>, font, text: &str, y, color| {
        gfx::text(c, font, text, Point::new(LEFT_X, y), Align::Center, color);
    };
    let lang = app.lang();
    left(c, Font::Big, lang.weekday_abbr(t.weekday), 70, theme.dim);
    left(c, Font::Day, &t.day.to_string(), 130, theme.fg);
    left(c, Font::Medium, &lang.month_year(&t), 192, theme.fg);
    if let Some(((rh, rm), (sh, sm))) = daylight(app) {
        let line = |c: &mut Canvas<'_>, sym, color, text: &str, y| {
            icons::symbol(c, sym, Point::new(SUN_X, y), 22, 2.0, color);
            gfx::text(
                c,
                Font::Body,
                text,
                Point::new(SUN_X + 19, y),
                Align::Left,
                theme.fg,
            );
        };
        line(
            c,
            Symbol::Sun,
            theme.caution,
            &format!("{rh:02}:{rm:02}"),
            237,
        );
        line(c, Symbol::Moon, theme.dim, &format!("{sh:02}:{sm:02}"), 267);
    }
    paint_month(c, t, lang, theme);
}

fn paint_month(c: &mut Canvas<'_>, t: LocalTime, lang: Language, theme: &Theme) {
    let col = |i: i32| GRID_X + COL_W * i;
    for i in 0..7u8 {
        gfx::text(
            c,
            Font::Body,
            lang.weekday_letters(i),
            Point::new(col(i32::from(i)), HEADER_Y),
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
            gfx::circle(c, at, 30, theme.accent);
            theme.on_accent
        } else if column >= 5 {
            theme.dim
        } else {
            theme.fg
        };
        gfx::text(c, Font::Body, &day.to_string(), at, Align::Center, color);
    }
}
