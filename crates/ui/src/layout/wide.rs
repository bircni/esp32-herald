//! The wide layout, for e-paper used in landscape (792x272).
//!
//! The dashboard has three columns: clock, date and the current weather on
//! the left, the next hours and days in the middle, the feed on the right.
//! Custom screens are further pages with their widgets side by side.

use embedded_graphics::prelude::*;

use super::paper::{self, Col, center, rule};
use crate::{
    app::{App, Showing},
    gfx::{self, Align, Canvas, Font},
    i18n::{Language, Text},
    model::{Env, NetState, ViewId},
    theme::Theme,
    views::{custom, icons},
    weather::{Weather, deg},
};

/// Size in pixels.
pub const W: i32 = 792;
pub const H: i32 = 272;

const LEFT: Col = Col { x: 0, w: 264 };
const MIDDLE: Col = Col { x: 264, w: 264 };
const RIGHT: Col = Col { x: 528, w: 264 };
const DOTS_Y: i32 = 263;

pub(crate) fn paint(app: &App, c: &mut Canvas<'_>, env: &Env, t: &Theme) {
    if app.showing(env) == Showing::WifiSetup {
        paint_setup(c, env, app.lang(), t);
        return;
    }
    match paper::page(app) {
        ViewId::Screen(slot) => {
            if let Some(entry) = app.screens.get(slot) {
                let area = custom::Area {
                    width: W,
                    bottom: DOTS_Y - 10,
                    portrait: false,
                    outlined: true,
                };
                custom::paint_entry(app, entry, c, env, t, area);
            }
            paper::paint_dots(app, c, t, (W / 2, DOTS_Y));
        }
        ViewId::Home | ViewId::Forecast | ViewId::Calendar => {
            paint_dashboard(app, c, env, t);
            paper::paint_dots(app, c, t, (MIDDLE.x + MIDDLE.w / 2, DOTS_Y));
        }
    }
}

/// A line down between two columns, from `top` to near the bottom.
fn divider(c: &mut Canvas<'_>, x: i32, top: i32, t: &Theme) {
    gfx::fill(c, gfx::rect(x - 1, top, 2, (H - 16 - top) as u32), t.fg);
}

fn paint_dashboard(app: &App, c: &mut Canvas<'_>, env: &Env, t: &Theme) {
    center(c, LEFT, Font::Clock, &paper::time_text(env), 58, t.fg);
    center(
        c,
        LEFT,
        Font::Big,
        &paper::date_text(env, true, app.lang()),
        124,
        t.fg,
    );
    rule(c, LEFT, 150, t);
    divider(c, MIDDLE.x, 16, t);
    divider(c, RIGHT.x, 16, t);

    if let Some(w) = &app.weather {
        paint_now(c, w, app.lang(), t);
        let hours_end = paper::paint_hours(c, MIDDLE, 8, w.upcoming(env.local.as_ref()), t);
        if paper::paint_days(c, MIDDLE, hours_end + 14, (w, app.lang()), t) {
            rule(c, MIDDLE, hours_end + 6, t);
        }
    } else {
        let lang = app.lang();
        center(
            c,
            LEFT,
            Font::Medium,
            lang.text(Text::NoWeatherYet),
            210,
            t.fg,
        );
        center(
            c,
            MIDDLE,
            Font::Medium,
            lang.text(Text::NoForecastYet),
            120,
            t.fg,
        );
    }
    paper::paint_feed(app, c, env, t, RIGHT, (8, H - 8));
}

/// Current weather, below the date.
fn paint_now(c: &mut Canvas<'_>, w: &Weather, lang: Language, t: &Theme) {
    icons::weather(c, Point::new(54, 194), 56, w.condition, w.is_day, 1.6, t.fg);
    gfx::text(
        c,
        Font::Huge,
        &deg(w.temp_c),
        Point::new(98, 188),
        Align::Left,
        t.fg,
    );
    let range = format!("{} / {}", deg(w.min_c), deg(w.max_c));
    gfx::text(
        c,
        Font::Body,
        &range,
        Point::new(100, 226),
        Align::Left,
        t.fg,
    );
    center(
        c,
        LEFT,
        Font::Medium,
        lang.condition(w.condition),
        254,
        t.fg,
    );
}

fn paint_setup(c: &mut Canvas<'_>, env: &Env, lang: Language, t: &Theme) {
    let NetState::Setup { ssid, password, ip } = &env.net else {
        return;
    };
    let all = Col { x: 0, w: W };
    let (join, open) = (Col { x: 0, w: W / 2 }, Col { x: W / 2, w: W / 2 });
    let say = |id| lang.text(id);
    center(c, all, Font::Large, say(Text::WifiSetup), 34, t.fg);
    rule(c, all, 62, t);
    center(c, join, Font::Body, say(Text::JoinNetwork), 104, t.fg);
    center(c, join, Font::Large, ssid, 140, t.fg);
    center(c, join, Font::Body, say(Text::Password), 186, t.fg);
    center(c, join, Font::Large, password, 220, t.fg);
    center(c, open, Font::Body, say(Text::OpenInBrowser), 104, t.fg);
    center(c, open, Font::Bold, &format!("http://{ip}"), 140, t.fg);
    center(c, open, Font::Body, say(Text::EnterHomeWifi), 186, t.fg);
    center(c, open, Font::Body, say(Text::Band), 212, t.fg);
    divider(c, W / 2, 80, t);
}
