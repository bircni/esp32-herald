//! The tall layout, for upright e-paper (272x792).
//!
//! One dashboard has everything stacked: clock, weather, the next hours,
//! the feed and the next days. Custom screens are further pages, with one
//! widget per row.

use embedded_graphics::prelude::*;

pub(crate) use super::paper::page;
pub use super::paper::pages;
use super::paper::{self, Col, DAYS_H, center, rule};
use crate::{
    app::{App, Showing},
    gfx::{self, Align, Canvas, Font},
    i18n::{Language, Text},
    model::{Env, NetState, ViewId},
    theme::Theme,
    views::{custom, icons},
    weather::{Weather, deg},
};

/// Upright size in pixels.
pub const W: i32 = 272;
pub const H: i32 = 792;

/// The whole width is one column.
const COL: Col = Col { x: 0, w: W };
/// Page dots sit at the very bottom.
const DOTS_Y: i32 = 783;

pub(crate) fn paint(app: &App, c: &mut Canvas<'_>, env: &Env, t: &Theme) {
    if app.showing(env) == Showing::WifiSetup {
        paint_setup(c, env, app.lang(), t);
        return;
    }
    match page(app) {
        ViewId::Screen(slot) => {
            if let Some(entry) = app.screens.get(slot) {
                let area = custom::Area {
                    width: W,
                    bottom: DOTS_Y - 12,
                    portrait: true,
                    outlined: true,
                };
                custom::paint_entry(app, entry, c, env, t, area);
            }
        }
        ViewId::Home | ViewId::Forecast | ViewId::Calendar => paint_dashboard(app, c, env, t),
    }
    paper::paint_dots(app, c, t, (W / 2, DOTS_Y));
}

fn paint_dashboard(app: &App, c: &mut Canvas<'_>, env: &Env, t: &Theme) {
    center(c, COL, Font::Clock, &paper::time_text(env), 66, t.fg);
    center(
        c,
        COL,
        Font::Big,
        &paper::date_text(env, false, app.lang()),
        138,
        t.fg,
    );
    rule(c, COL, 166, t);

    let (top, bottom) = app.weather.as_ref().map_or((176, DOTS_Y - 14), |w| {
        paint_now(c, w, app.lang(), t);
        let hours_end = paper::paint_hours(c, COL, 304, w.upcoming(env.local.as_ref()), t);
        rule(c, COL, hours_end, t);
        let days_top = DOTS_Y - 14 - DAYS_H;
        let days_top = if paper::paint_days(c, COL, days_top, (w, app.lang()), t) {
            rule(c, COL, days_top, t);
            days_top
        } else {
            DOTS_Y
        };
        (hours_end + 8, days_top - 4)
    });
    paper::paint_feed(app, c, env, t, COL, (top, bottom));
}

/// Current weather, below the date.
fn paint_now(c: &mut Canvas<'_>, w: &Weather, lang: Language, t: &Theme) {
    icons::weather(c, Point::new(66, 222), 72, w.condition, w.is_day, 1.5, t.fg);
    gfx::text(
        c,
        Font::Huge,
        &deg(w.temp_c),
        Point::new(118, 214),
        Align::Left,
        t.fg,
    );
    let range = format!("{} / {}", deg(w.min_c), deg(w.max_c));
    gfx::text(
        c,
        Font::Body,
        &range,
        Point::new(120, 252),
        Align::Left,
        t.fg,
    );
    center(c, COL, Font::Medium, lang.condition(w.condition), 284, t.fg);
}

fn paint_setup(c: &mut Canvas<'_>, env: &Env, lang: Language, t: &Theme) {
    let NetState::Setup { ssid, password, ip } = &env.net else {
        return;
    };
    let say = |id| lang.text(id);
    center(c, COL, Font::Large, say(Text::WifiSetup), 60, t.fg);
    rule(c, COL, 96, t);
    center(c, COL, Font::Body, say(Text::JoinNetwork), 140, t.fg);
    center(c, COL, Font::Large, ssid, 176, t.fg);
    center(c, COL, Font::Body, say(Text::Password), 222, t.fg);
    center(c, COL, Font::Large, password, 256, t.fg);
    center(c, COL, Font::Body, say(Text::OpenInBrowser), 320, t.fg);
    center(c, COL, Font::Bold, &format!("http://{ip}"), 354, t.fg);
    center(c, COL, Font::Body, say(Text::EnterHomeWifi), 418, t.fg);
    center(c, COL, Font::Body, say(Text::Band), 442, t.fg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Profile, Settings};

    #[test]
    fn next_and_previous_page_through_screens() {
        let mut a = App::new(Profile::CROWPANEL_579, Settings::default(), None);
        let screen = serde_json::from_str(r#"{"widgets":[{"type":"value","value":"1"}]}"#)
            .unwrap_or_else(|e| panic!("{e}"));
        a.put_screen("energy", screen, 0)
            .unwrap_or_else(|e| panic!("{e}"));
        let e = Env::default();
        assert_eq!(pages(&a).len(), 2);
        a.act(Action::Next, &e);
        assert!(matches!(a.view(), ViewId::Screen(_)));
        a.act(Action::Next, &e);
        assert_eq!(a.view(), ViewId::Home, "wraps around");
        a.act(Action::Previous, &e);
        assert!(matches!(a.view(), ViewId::Screen(_)));
        a.act(Action::Home, &e);
        assert_eq!(a.view(), ViewId::Home);
        assert!(!a.take_refresh());
        a.act(Action::Refresh, &e);
        assert!(a.take_refresh());
        assert!(!a.take_refresh(), "taken once");
        a.act(Action::Menu, &e);
        assert_eq!(a.view(), ViewId::Home, "no menu here");
        // Forecast and calendar are part of the dashboard.
        a.set_view(ViewId::Forecast, 0);
        assert_eq!(page(&a), ViewId::Home);
    }
}
