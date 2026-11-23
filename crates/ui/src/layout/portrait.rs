//! The tall layout, for upright e-paper (272x792).
//!
//! One dashboard has everything stacked: clock, weather, the next hours,
//! the feed and the next days. Custom screens are further pages, with one
//! widget per row.
//!
//! E-paper cannot animate and every refresh takes a moment, so there is no
//! menu and nothing moves; `Next` and `Previous` page through.

use embedded_graphics::{pixelcolor::Rgb565, prelude::*};

use crate::{
    app::{App, Showing},
    feed::{self, Item},
    gfx::{self, Align, Canvas, Font},
    model::{Env, ItemColor, NetState, ViewId, weekday_abbr},
    theme::Theme,
    views::{custom, home, icons},
    weather::{Hour, Weather, deg},
};

/// Upright size in pixels.
pub const W: i32 = 272;
pub const H: i32 = 792;

const MARGIN: i32 = 16;
/// Page dots sit at the very bottom.
const DOTS_Y: i32 = 783;

/// The dashboard, then every custom screen.
#[must_use]
pub fn pages(app: &App) -> Vec<ViewId> {
    std::iter::once(ViewId::Home)
        .chain(app.screens.ordered().into_iter().map(ViewId::Screen))
        .collect()
}

/// The page shown for the app's current view: forecast and calendar are
/// part of the dashboard here.
pub(crate) fn page(app: &App) -> ViewId {
    match app.view {
        ViewId::Screen(slot) if app.screens.get(slot).is_some() => app.view,
        ViewId::Home | ViewId::Forecast | ViewId::Calendar | ViewId::Screen(_) => ViewId::Home,
    }
}

fn center(c: &mut Canvas<'_>, font: Font, text: &str, y: i32, color: Rgb565) {
    let text = gfx::shorten(font, text, W - 2 * MARGIN);
    gfx::text(c, font, &text, Point::new(W / 2, y), Align::Center, color);
}

fn rule(c: &mut Canvas<'_>, y: i32, t: &Theme) {
    gfx::fill(c, gfx::rect(MARGIN, y, (W - 2 * MARGIN) as u32, 2), t.fg);
}

pub(crate) fn paint(app: &App, c: &mut Canvas<'_>, env: &Env, t: &Theme) {
    if app.showing(env) == Showing::WifiSetup {
        paint_setup(c, env, t);
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
    paint_dots(app, c, t);
}

fn paint_dots(app: &App, c: &mut Canvas<'_>, t: &Theme) {
    let order = pages(app);
    if order.len() < 2 {
        return;
    }
    let current = page(app);
    let n = order.len() as i32;
    let left = W / 2 - 7 * (n - 1);
    for (i, v) in order.iter().enumerate() {
        let at = Point::new(left + 14 * i as i32, DOTS_Y);
        if *v == current {
            gfx::circle(c, at, 8, t.fg);
        } else {
            gfx::ring(c, at, 8, 1, t.fg);
        }
    }
}

fn paint_dashboard(app: &App, c: &mut Canvas<'_>, env: &Env, t: &Theme) {
    let time = env.local.map_or_else(
        || "--:--".to_owned(),
        |l| format!("{:02}:{:02}", l.hour, l.minute),
    );
    center(c, Font::Clock, &time, 66, t.fg);
    let date = env.local.map_or_else(
        || home::date_line(env),
        |l| {
            let month = l.month_name().get(..3).unwrap_or_default();
            format!("{} {} {month}", l.weekday_name(), l.day)
        },
    );
    center(c, Font::Big, &date, 138, t.fg);
    rule(c, 166, t);

    let (top, bottom) = app.weather.as_ref().map_or((176, DOTS_Y - 14), |w| {
        paint_now(c, w, t);
        let hours_end = paint_hours(c, w.upcoming(env.local.as_ref()), t);
        let days_top = paint_days(c, w, t);
        rule(c, hours_end, t);
        if days_top < DOTS_Y {
            rule(c, days_top, t);
        }
        (hours_end + 8, days_top - 4)
    });
    paint_feed(app, c, env, t, top, bottom);
}

/// Current weather, below the date.
fn paint_now(c: &mut Canvas<'_>, w: &Weather, t: &Theme) {
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
    center(c, Font::Medium, w.condition.label(), 284, t.fg);
}

/// The next hours: time, icon, temperature, rain. Returns
/// where the section ends.
fn paint_hours(c: &mut Canvas<'_>, upcoming: &[Hour], t: &Theme) -> i32 {
    let top = 304;
    // Hourly forecasts show every second hour; coarser ones every entry.
    let hourly = upcoming
        .windows(2)
        .next()
        .is_some_and(|p| (p[1].hour + 24 - p[0].hour) % 24 == 1);
    let step = if hourly { 2 } else { 1 };
    let hours: Vec<_> = upcoming.iter().step_by(step).take(6).collect();
    if hours.is_empty() {
        return top;
    }
    let col = (W - 2 * MARGIN) / hours.len() as i32;
    for (i, h) in hours.iter().enumerate() {
        let x = MARGIN + col * i as i32 + col / 2;
        let at = |y| Point::new(x, y);
        gfx::text(
            c,
            Font::Small,
            &format!("{:02}", h.hour),
            at(top + 10),
            Align::Center,
            t.fg,
        );
        icons::weather(c, at(top + 38), 28, h.condition, h.is_day, 1.8, t.fg);
        gfx::text(
            c,
            Font::Body,
            &deg(h.temp_c),
            at(top + 68),
            Align::Center,
            t.fg,
        );
        if h.rain_pct > 0 {
            gfx::text(
                c,
                Font::Small,
                &format!("{}%", h.rain_pct),
                at(top + 88),
                Align::Center,
                t.fg,
            );
        }
    }
    top + 102
}

/// The next days along the bottom. Returns where the section starts.
fn paint_days(c: &mut Canvas<'_>, w: &Weather, t: &Theme) -> i32 {
    let top = DOTS_Y - 14 - 82;
    if w.days.is_empty() {
        return DOTS_Y;
    }
    let col = (W - 2 * MARGIN) / w.days.len() as i32;
    for (i, d) in w.days.iter().enumerate() {
        let x = MARGIN + col * i as i32 + col / 2;
        let at = |y| Point::new(x, y);
        gfx::text(
            c,
            Font::Bold,
            weekday_abbr(d.weekday),
            at(top + 16),
            Align::Center,
            t.fg,
        );
        icons::weather(c, at(top + 44), 28, d.condition, true, 1.8, t.fg);
        let range = format!("{} {}", deg(d.max_c), deg(d.min_c));
        gfx::text(c, Font::Small, &range, at(top + 72), Align::Center, t.fg);
    }
    top
}

fn paint_feed(app: &App, c: &mut Canvas<'_>, env: &Env, t: &Theme, top: i32, bottom: i32) {
    let items = app.feed.items();
    if items.is_empty() {
        center(
            c,
            Font::Medium,
            "All quiet",
            i32::midpoint(top, bottom),
            t.fg,
        );
        return;
    }
    // Rows grow with their text; whatever does not fit is counted below.
    let rows: Vec<(&Item, Vec<String>)> =
        items.iter().map(|item| (item, item_lines(item))).collect();
    let mut y = top;
    for (i, (item, lines)) in rows.iter().enumerate() {
        let h = row_height(lines.len());
        let left = rows.len() - i - 1;
        let room = if left > 0 { bottom - MORE_H } else { bottom };
        if y + h > room {
            let more = format!("+{} more", rows.len() - i);
            gfx::text(
                c,
                Font::Medium,
                &more,
                Point::new(TEXT_X, y + MORE_H / 2),
                Align::Left,
                t.fg,
            );
            return;
        }
        paint_item(c, item, lines, env, t, y, h);
        y += h;
    }
}

/// Feed text starts here, right of the dot or icon.
const TEXT_X: i32 = MARGIN + 28;
const LINE_H: i32 = 22;
/// The "+N more" line.
const MORE_H: i32 = 30;

/// Without colour, urgency shows as weight: red and yellow items are bold.
const fn item_font(color: ItemColor) -> Font {
    match color {
        ItemColor::Red | ItemColor::Yellow => Font::Bold,
        ItemColor::Blue | ItemColor::Green | ItemColor::Grey => Font::Medium,
    }
}

/// The item's text in at most two lines.
fn item_lines(item: &Item) -> Vec<String> {
    gfx::wrap(item_font(item.color), &item.text, W - MARGIN - TEXT_X, 2)
}

/// Text lines, the age line below, and some air.
const fn row_height(lines: usize) -> i32 {
    LINE_H * (lines as i32 + 1) + 6
}

/// One feed row from `top`, `h` high. Red items are white on black.
fn paint_item(
    c: &mut Canvas<'_>,
    item: &Item,
    lines: &[String],
    env: &Env,
    t: &Theme,
    top: i32,
    h: i32,
) {
    let fg = if item.color == ItemColor::Red {
        let bar = gfx::rect(
            MARGIN - 6,
            top + 3,
            (W - 2 * MARGIN + 12) as u32,
            (h - 6) as u32,
        );
        gfx::round_rect(c, bar, 6, t.fg);
        t.bg
    } else {
        t.fg
    };
    let first = top + 3 + LINE_H / 2 + 1;
    let icon_at = Point::new(MARGIN + 10, first);
    match item.icon {
        Some(sym) => icons::symbol(c, sym, icon_at, 20, 2.2, fg),
        None => gfx::circle(c, icon_at, 10, fg),
    }
    let font = item_font(item.color);
    let mut y = first;
    for line in lines {
        gfx::text(c, font, line, Point::new(TEXT_X, y), Align::Left, fg);
        y += LINE_H;
    }
    let below = if item.detail.is_empty() {
        feed::age(item.age_ms(env.now_ms))
    } else {
        item.detail.clone()
    };
    let below = gfx::shorten(Font::Small, &below, W - MARGIN - TEXT_X);
    gfx::text(
        c,
        Font::Small,
        &below,
        Point::new(TEXT_X, y),
        Align::Left,
        fg,
    );
}

fn paint_setup(c: &mut Canvas<'_>, env: &Env, t: &Theme) {
    let NetState::Setup { ssid, password, ip } = &env.net else {
        return;
    };
    center(c, Font::Large, "WiFi setup", 60, t.fg);
    rule(c, 96, t);
    center(c, Font::Body, "1. Join this WiFi network", 140, t.fg);
    center(c, Font::Large, ssid, 176, t.fg);
    center(c, Font::Body, "Password", 222, t.fg);
    center(c, Font::Large, password, 256, t.fg);
    center(c, Font::Body, "2. Open in your browser", 320, t.fg);
    center(c, Font::Bold, &format!("http://{ip}"), 354, t.fg);
    center(c, Font::Body, "3. Enter your home WiFi", 418, t.fg);
    center(c, Font::Body, "(2.4 GHz)", 442, t.fg);
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
