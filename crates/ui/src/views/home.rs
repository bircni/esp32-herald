//! The home view: clock and weather on top, the feed below. Laid out like
//! the web app's mirror (`drawHome` in `web/index.html`).

use embedded_graphics::{prelude::*, primitives::Rectangle};

use super::{Region, icons, region};
use crate::{
    app::App,
    feed::Item,
    gfx::{self, Align, Canvas, Font},
    i18n::{Language, Text},
    model::{Env, NetState},
    theme::Theme,
    weather::deg,
};

const CLOCK: Rectangle = gfx::rect(0, 8, 300, 104);
const DATE: Rectangle = gfx::rect(0, 112, 300, 32);
// Wide enough for "Comes in over MQTT" centred at x = 400.
const WEATHER: Rectangle = gfx::rect(300, 8, 180, 136);
const DIVIDER: Rectangle = gfx::rect(14, 148, 452, 1);

pub const ROWS: usize = 5;
/// Centre of the first feed row, and the distance between rows.
const ROW_Y: i32 = 171;
const ROW_H: i32 = 29;
/// Feed rows: text from here, the age or detail ending at `ROW_RIGHT`.
const ROW_TEXT_X: i32 = 38;
const ROW_RIGHT: i32 = 466;
/// Fonts the feed may use, largest first: long texts make the whole feed
/// smaller before any is cut. All rows share one.
const ROW_FONTS: [Font; 3] = [Font::Big, Font::Medium, Font::Body];

/// The age or detail goes down a size with the text.
const fn right_font(font: Font) -> Font {
    if matches!(font, Font::Big) {
        Font::Medium
    } else {
        Font::Body
    }
}

/// Room for an item's text in `font`, left of its age or detail.
fn text_width(item: &Item, font: Font, now_ms: u64, lang: Language) -> i32 {
    let right = gfx::text_width(right_font(font), &right_text(item, now_ms, lang));
    ROW_RIGHT - right - 12 - ROW_TEXT_X
}

/// The largest font in which every shown item fits.
fn rows_font(app: &App, env: &Env) -> Font {
    let shown: Vec<&Item> = (0..ROWS)
        .filter_map(|i| match row(app, i) {
            Row::Item(item) => Some(item),
            Row::More(_) | Row::Quiet | Row::Empty => None,
        })
        .collect();
    ROW_FONTS
        .into_iter()
        .find(|f| {
            shown.iter().all(|item| {
                gfx::text_width(*f, &item.text) <= text_width(item, *f, env.now_ms, app.lang())
            })
        })
        .unwrap_or(Font::Body)
}

/// Taller than the row spacing: text in the row font reaches a few pixels
/// beyond it, and overlapping regions are fine (each repaint draws the
/// whole view, clipped).
const ROW_REGION_H: i32 = 34;

const fn row_rect(i: usize) -> Rectangle {
    let center = ROW_Y + ROW_H * i as i32;
    gfx::rect(0, center - ROW_REGION_H / 2, 480, ROW_REGION_H as u32)
}

fn time_text(env: &Env) -> String {
    env.local.map_or_else(
        || "--:--".to_owned(),
        |t| format!("{:02}:{:02}", t.hour, t.minute),
    )
}

pub(crate) fn date_line(env: &Env, lang: Language) -> String {
    match env.local {
        Some(t) => lang.date(&t),
        None if matches!(env.net, NetState::Connected { .. }) => {
            lang.text(Text::WaitingForTimeSync).into()
        }
        None => lang.text(Text::NoWifiNoTime).into(),
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
        // "All quiet" sits in the middle of the feed area.
        return if i == 2 { Row::Quiet } else { Row::Empty };
    }
    if items.len() > ROWS && i == ROWS - 1 {
        return Row::More(items.len() - (ROWS - 1));
    }
    items.get(i).map_or(Row::Empty, Row::Item)
}

fn right_text(item: &Item, now_ms: u64, lang: Language) -> String {
    if item.detail.is_empty() {
        lang.age(item.age_ms(now_ms))
    } else {
        item.detail.clone()
    }
}

pub(crate) fn regions(app: &App, env: &Env) -> Vec<Region> {
    let mut regions = vec![
        region(CLOCK, &[gfx::str_key(&time_text(env))]),
        region(DATE, &[gfx::str_key(&date_line(env, app.lang()))]),
    ];
    let mut weather = vec![app.weather.is_some().into()];
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

    let font = rows_font(app, env) as i64;
    for i in 0..ROWS {
        let key = match row(app, i) {
            Row::Item(item) => vec![
                1,
                font,
                gfx::str_key(&item.text),
                gfx::str_key(&right_text(item, env.now_ms, app.lang())),
                item.color as i64,
                item.icon.map_or(-1, |s| s as i64),
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
    gfx::text(
        c,
        Font::Clock,
        &time_text(env),
        Point::new(14, 62),
        Align::Left,
        theme.fg,
    );
    let lang = app.lang();
    let date = gfx::shorten(
        Font::Big,
        &date_line(env, lang),
        DATE.size.width as i32 - 24,
    );
    gfx::text(
        c,
        Font::Big,
        &date,
        Point::new(18, 128),
        Align::Left,
        if env.local.is_some() {
            theme.fg
        } else {
            theme.dim
        },
    );
    paint_weather(app, c, theme);
    gfx::fill(c, DIVIDER, theme.rule);

    let font = rows_font(app, env);
    for i in 0..ROWS {
        let y = ROW_Y + ROW_H * i as i32;
        match row(app, i) {
            Row::Item(item) => {
                let color = theme.item(item.color);
                match item.icon {
                    Some(sym) => icons::symbol(c, sym, Point::new(22, y), 18, 2.2, color),
                    None => gfx::circle(c, Point::new(22, y), 12, color),
                }
                let right = right_text(item, env.now_ms, lang);
                let text = gfx::shorten(font, &item.text, text_width(item, font, env.now_ms, lang));
                gfx::text(
                    c,
                    font,
                    &text,
                    Point::new(ROW_TEXT_X, y),
                    Align::Left,
                    theme.fg,
                );
                gfx::text(
                    c,
                    right_font(font),
                    &right,
                    Point::new(ROW_RIGHT, y),
                    Align::Right,
                    theme.dim,
                );
            }
            Row::More(n) => gfx::text(
                c,
                Font::Medium,
                &lang.more(n),
                Point::new(ROW_TEXT_X, y),
                Align::Left,
                theme.dim,
            ),
            Row::Quiet => gfx::text(
                c,
                Font::Medium,
                lang.text(Text::AllQuiet),
                Point::new(240, y),
                Align::Center,
                theme.dim,
            ),
            Row::Empty => {}
        }
    }
}

fn paint_weather(app: &App, c: &mut Canvas<'_>, theme: &Theme) {
    let centered = |c: &mut Canvas<'_>, text: &str, y, color| {
        gfx::text(
            c,
            Font::Body,
            text,
            Point::new(400, y),
            Align::Center,
            color,
        );
    };
    let Some(w) = &app.weather else {
        centered(c, app.lang().text(Text::NoWeatherYet), 60, theme.dim);
        centered(c, app.lang().text(Text::ComesOverMqtt), 82, theme.dim);
        return;
    };
    icons::weather(
        c,
        Point::new(354, 46),
        48,
        w.condition,
        w.is_day,
        1.6,
        theme.dim,
    );
    gfx::text(
        c,
        Font::Temp,
        &deg(w.temp_c),
        Point::new(386, 48),
        Align::Left,
        theme.fg,
    );
    centered(c, app.lang().condition(w.condition), 98, theme.fg);
    centered(
        c,
        &format!("{} / {}", deg(w.min_c), deg(w.max_c)),
        122,
        theme.dim,
    );
}
