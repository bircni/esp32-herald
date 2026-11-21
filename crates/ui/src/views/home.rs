//! The home view: clock and weather on top, the feed below.

use embedded_graphics::{prelude::*, primitives::Rectangle};

use super::{icons, region, Region};
use crate::{
    app::App,
    digits::{self, two, Size7},
    feed::{self, Item},
    gfx::{self, Align, Canvas, Font},
    model::{Env, NetState},
    theme::Theme,
    weather::deg,
};

const DIGIT: Size7 = Size7 {
    w: 44,
    h: 92,
    t: 11,
    gap: 2,
};
const DIGIT_Y: i32 = 14;
const DIGIT_X: [i32; 4] = [16, 70, 140, 194];
const COLON_X: i32 = 127;
const DATE: Rectangle = gfx::rect(0, 112, 252, 30);
const WEATHER: Rectangle = gfx::rect(252, 6, 228, 140);
const ICON: Point = Point::new(302, 58);
const DIVIDER: Rectangle = gfx::rect(16, 150, 448, 2);

pub const ROWS: usize = 5;
const ROW_Y: i32 = 157;
const ROW_H: i32 = 29;

fn row_rect(i: usize) -> Rectangle {
    gfx::rect(0, ROW_Y + ROW_H * i as i32, 480, ROW_H as u32)
}

fn time_digits(env: &Env) -> [Option<u8>; 4] {
    match env.local {
        Some(t) => {
            let (h, m) = (two(u32::from(t.hour)), two(u32::from(t.minute)));
            [Some(h[0]), Some(h[1]), Some(m[0]), Some(m[1])]
        }
        None => [None; 4],
    }
}

fn date_line(env: &Env) -> String {
    match env.local {
        Some(t) => format!(
            "{} {} {}",
            crate::model::weekday_abbr(t.weekday),
            t.day,
            t.month_name()
        ),
        None if matches!(env.net, NetState::Connected { .. }) => "Waiting for time sync".into(),
        None => "No WiFi, time not set".into(),
    }
}

/// What a feed row shows: an item, a "+N more" line, the empty-feed
/// message, or nothing.
enum Row<'a> {
    Item(&'a Item),
    More(usize),
    Quiet,
    Empty,
}

fn row(app: &App, i: usize) -> Row<'_> {
    let items = app.feed.items();
    if items.is_empty() {
        return if i == ROWS / 2 {
            Row::Quiet
        } else {
            Row::Empty
        };
    }
    if items.len() > ROWS && i == ROWS - 1 {
        return Row::More(items.len() - (ROWS - 1));
    }
    items.get(i).map_or(Row::Empty, Row::Item)
}

fn right_text(item: &Item, now_ms: u64) -> String {
    if item.detail.is_empty() {
        feed::age(item.age_ms(now_ms))
    } else {
        item.detail.clone()
    }
}

pub(crate) fn regions(app: &App, env: &Env) -> Vec<Region> {
    let mut regions: Vec<Region> = time_digits(env)
        .into_iter()
        .enumerate()
        .map(|(i, d)| {
            region(
                digits::digit_rect(Point::new(DIGIT_X[i], DIGIT_Y), DIGIT),
                &[d.map_or(-1, i64::from)],
            )
        })
        .collect();
    regions.push(region(DATE, &[gfx::str_key(&date_line(env))]));

    let mut weather = vec![gfx::str_key(&app.settings.location)];
    if let Some(w) = &app.weather {
        weather.extend([
            w.temp_c.round() as i64,
            w.min_c.round() as i64,
            w.max_c.round() as i64,
            w.condition as i64,
            i64::from(w.is_day),
        ]);
    }
    regions.push(region(WEATHER, &weather));

    for i in 0..ROWS {
        let key = match row(app, i) {
            Row::Item(item) => vec![
                1,
                gfx::str_key(&item.text),
                gfx::str_key(&right_text(item, env.now_ms)),
                item.color as i64,
            ],
            Row::More(n) => vec![2, n as i64],
            Row::Quiet => vec![3],
            Row::Empty => vec![4],
        };
        regions.push(region(row_rect(i), &key));
    }
    regions
}

pub(crate) fn paint(app: &App, c: &mut Canvas<'_>, env: &Env, theme: &Theme) {
    for (i, d) in time_digits(env).into_iter().enumerate() {
        digits::draw_digit(
            c,
            Point::new(DIGIT_X[i], DIGIT_Y),
            DIGIT,
            d,
            theme.fg,
            theme.bg,
        );
    }
    digits::draw_colon(c, COLON_X, DIGIT_Y, DIGIT, theme.fg);
    gfx::text(
        c,
        Font::Medium,
        &date_line(env),
        Point::new(127, 128),
        Align::Center,
        if env.local.is_some() {
            theme.fg
        } else {
            theme.dim
        },
    );

    paint_weather(app, c, theme);
    gfx::fill(c, DIVIDER, theme.panel);

    for i in 0..ROWS {
        let r = row_rect(i);
        let y = r.center().y;
        match row(app, i) {
            Row::Item(item) => {
                gfx::circle(c, Point::new(26, y), 14, theme.item(item.color));
                let font = if item.text.chars().count() <= 26 {
                    Font::Medium
                } else {
                    Font::Small
                };
                gfx::text(
                    c,
                    font,
                    &item.text,
                    Point::new(44, y),
                    Align::Left,
                    theme.fg,
                );
                gfx::text(
                    c,
                    Font::Small,
                    &right_text(item, env.now_ms),
                    Point::new(464, y),
                    Align::Right,
                    theme.dim,
                );
            }
            Row::More(n) => gfx::text(
                c,
                Font::Small,
                &format!("+{n} more"),
                Point::new(44, y),
                Align::Left,
                theme.dim,
            ),
            Row::Quiet => gfx::text(
                c,
                Font::Medium,
                "All quiet",
                Point::new(240, y),
                Align::Center,
                theme.dim,
            ),
            Row::Empty => {}
        }
    }
}

fn paint_weather(app: &App, c: &mut Canvas<'_>, theme: &Theme) {
    let cx = WEATHER.center().x;
    if !app.settings.has_location() {
        gfx::text(
            c,
            Font::Small,
            "No weather location",
            Point::new(cx, 60),
            Align::Center,
            theme.dim,
        );
        gfx::text(
            c,
            Font::Small,
            "Set one on the web page",
            Point::new(cx, 82),
            Align::Center,
            theme.dim,
        );
        return;
    }
    let Some(w) = &app.weather else {
        gfx::text(
            c,
            Font::Small,
            &app.settings.location,
            Point::new(cx, 60),
            Align::Center,
            theme.fg,
        );
        gfx::text(
            c,
            Font::Small,
            "Loading weather...",
            Point::new(cx, 82),
            Align::Center,
            theme.dim,
        );
        return;
    };
    icons::weather(c, ICON, 0.5, w.condition, w.is_day, theme, theme.bg);
    gfx::text(
        c,
        Font::Huge,
        &deg(w.temp_c),
        Point::new(352, 58),
        Align::Left,
        theme.fg,
    );
    gfx::text(
        c,
        Font::Small,
        w.condition.label(),
        Point::new(cx, 106),
        Align::Center,
        theme.fg,
    );
    let place: String = app.settings.location.chars().take(14).collect();
    let line = format!("{} / {}  \u{b7}  {place}", deg(w.min_c), deg(w.max_c));
    gfx::text(
        c,
        Font::Small,
        &line,
        Point::new(cx, 128),
        Align::Center,
        theme.dim,
    );
}
