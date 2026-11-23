//! Building blocks of the e-paper layouts ([`super::portrait`] and
//! [`super::wide`]): the pages, and the feed, forecast and page dots drawn
//! into a column of any width.
//!
//! E-paper cannot animate and every refresh takes a moment, so there is no
//! menu and nothing moves; `Next` and `Previous` page through the dashboard
//! and the custom screens.

use embedded_graphics::{pixelcolor::Rgb565, prelude::*};

use crate::{
    app::App,
    feed::Item,
    gfx::{self, Align, Canvas, Font},
    i18n::{Language, Text},
    model::{Env, ItemColor, ViewId},
    theme::Theme,
    views::{home, icons},
    weather::{Hour, Weather, deg},
};

/// Space between a column's edge and its content.
const MARGIN: i32 = 16;
const LINE_H: i32 = 22;
/// The "+N more" line.
const MORE_H: i32 = 30;

/// A vertical strip of the screen: its left edge and width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Col {
    pub x: i32,
    pub w: i32,
}

impl Col {
    const fn center(self) -> i32 {
        self.x + self.w / 2
    }

    /// Width inside the margins.
    const fn inner(self) -> i32 {
        self.w - 2 * MARGIN
    }

    /// Feed text starts here, right of the dot or icon.
    const fn text_x(self) -> i32 {
        self.x + MARGIN + 28
    }

    const fn text_w(self) -> i32 {
        self.x + self.w - MARGIN - self.text_x()
    }
}

/// The dashboard, then every custom screen.
#[must_use]
pub fn pages(app: &App) -> Vec<ViewId> {
    std::iter::once(ViewId::Home)
        .chain(app.screens.ordered().into_iter().map(ViewId::Screen))
        .collect()
}

/// The page shown for the app's current view: forecast and calendar are
/// part of the dashboard here.
pub fn page(app: &App) -> ViewId {
    match app.view {
        ViewId::Screen(slot) if app.screens.get(slot).is_some() => app.view,
        ViewId::Home | ViewId::Forecast | ViewId::Calendar | ViewId::Screen(_) => ViewId::Home,
    }
}

pub fn center(c: &mut Canvas<'_>, col: Col, font: Font, text: &str, y: i32, color: Rgb565) {
    let text = gfx::shorten(font, text, col.inner());
    gfx::text(
        c,
        font,
        &text,
        Point::new(col.center(), y),
        Align::Center,
        color,
    );
}

/// A line across the column.
pub fn rule(c: &mut Canvas<'_>, col: Col, y: i32, t: &Theme) {
    gfx::fill(c, gfx::rect(col.x + MARGIN, y, col.inner() as u32, 2), t.fg);
}

pub fn time_text(env: &Env) -> String {
    env.local.map_or_else(
        || "--:--".to_owned(),
        |l| format!("{:02}:{:02}", l.hour, l.minute),
    )
}

/// "Monday 23 Nov" (with `short`: "Mon 23 Nov"), or why the clock is not
/// set.
pub fn date_text(env: &Env, short: bool, lang: Language) -> String {
    env.local.map_or_else(
        || home::date_line(env, lang),
        |l| lang.date_with_weekday(&l, short),
    )
}

/// Dots for the pages, centred on `mid` in `row`, when there is more than
/// one.
pub fn paint_dots(app: &App, c: &mut Canvas<'_>, t: &Theme, (mid, row): (i32, i32)) {
    let order = pages(app);
    if order.len() < 2 {
        return;
    }
    let current = page(app);
    let count = order.len() as i32;
    let left = mid - 7 * (count - 1);
    for (i, v) in order.iter().enumerate() {
        let at = Point::new(left + 14 * i as i32, row);
        if *v == current {
            gfx::circle(c, at, 8, t.fg);
        } else {
            gfx::ring(c, at, 8, 1, t.fg);
        }
    }
}

/// The next hours from `top`: time, icon, temperature, rain. Returns where
/// the section ends.
pub fn paint_hours(c: &mut Canvas<'_>, col: Col, top: i32, upcoming: &[Hour], t: &Theme) -> i32 {
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
    let cell = col.inner() / hours.len() as i32;
    for (i, h) in hours.iter().enumerate() {
        let x = col.x + MARGIN + cell * i as i32 + cell / 2;
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

/// Height of [`paint_days`].
pub const DAYS_H: i32 = 82;

/// The next days from `top`: weekday, icon, high and low. Draws nothing
/// without days and returns whether it drew.
pub fn paint_days(
    c: &mut Canvas<'_>,
    col: Col,
    top: i32,
    (w, lang): (&Weather, Language),
    t: &Theme,
) -> bool {
    if w.days.is_empty() {
        return false;
    }
    let cell = col.inner() / w.days.len() as i32;
    for (i, d) in w.days.iter().enumerate() {
        let x = col.x + MARGIN + cell * i as i32 + cell / 2;
        let at = |y| Point::new(x, y);
        gfx::text(
            c,
            Font::Bold,
            lang.weekday_abbr(d.weekday),
            at(top + 16),
            Align::Center,
            t.fg,
        );
        icons::weather(c, at(top + 44), 28, d.condition, true, 1.8, t.fg);
        let range = format!("{} {}", deg(d.max_c), deg(d.min_c));
        gfx::text(c, Font::Small, &range, at(top + 72), Align::Center, t.fg);
    }
    true
}

/// The feed between `top` and `bottom`. Rows grow with their text (up to
/// two lines); whatever does not fit is counted at the end.
pub fn paint_feed(
    app: &App,
    c: &mut Canvas<'_>,
    env: &Env,
    t: &Theme,
    col: Col,
    (top, bottom): (i32, i32),
) {
    let lang = app.lang();
    let items = app.feed.items();
    if items.is_empty() {
        center(
            c,
            col,
            Font::Medium,
            lang.text(Text::AllQuiet),
            i32::midpoint(top, bottom),
            t.fg,
        );
        return;
    }
    let rows: Vec<(&Item, Vec<String>)> = items
        .iter()
        .map(|item| (item, item_lines(item, col)))
        .collect();
    let mut y = top;
    for (i, (item, lines)) in rows.iter().enumerate() {
        let h = row_height(lines.len());
        let left = rows.len() - i - 1;
        let room = if left > 0 { bottom - MORE_H } else { bottom };
        if y + h > room {
            let more = lang.more(rows.len() - i);
            gfx::text(
                c,
                Font::Medium,
                &more,
                Point::new(col.text_x(), y + MORE_H / 2),
                Align::Left,
                t.fg,
            );
            return;
        }
        paint_item(c, col, item, lines, (env, lang), t, y);
        y += h;
    }
}

/// All feed text looks the same; urgency shows in the dot.
const ITEM_FONT: Font = Font::Medium;

/// Without colour, red and yellow items get a filled dot, the others an
/// outline.
const fn urgent(color: ItemColor) -> bool {
    match color {
        ItemColor::Red | ItemColor::Yellow => true,
        ItemColor::Blue | ItemColor::Green | ItemColor::Grey => false,
    }
}

/// The item's text in at most two lines.
fn item_lines(item: &Item, col: Col) -> Vec<String> {
    gfx::wrap(ITEM_FONT, &item.text, col.text_w(), 2)
}

/// Text lines, the age line below, and some air.
const fn row_height(lines: usize) -> i32 {
    LINE_H * (lines as i32 + 1) + 6
}

/// One feed row from `top`.
fn paint_item(
    c: &mut Canvas<'_>,
    col: Col,
    item: &Item,
    lines: &[String],
    (env, lang): (&Env, Language),
    t: &Theme,
    top: i32,
) {
    let fg = t.fg;
    let first = top + 3 + LINE_H / 2 + 1;
    let icon_at = Point::new(col.x + MARGIN + 10, first);
    match item.icon {
        Some(sym) => icons::symbol(c, sym, icon_at, 20, 2.2, fg),
        None if urgent(item.color) => gfx::circle(c, icon_at, 11, fg),
        None => gfx::ring(c, icon_at, 11, 2, fg),
    }
    let mut y = first;
    for line in lines {
        gfx::text(
            c,
            ITEM_FONT,
            line,
            Point::new(col.text_x(), y),
            Align::Left,
            fg,
        );
        y += LINE_H;
    }
    let below = if item.detail.is_empty() {
        lang.age(item.age_ms(env.now_ms))
    } else {
        item.detail.clone()
    };
    let below = gfx::shorten(Font::Small, &below, col.text_w());
    gfx::text(
        c,
        Font::Small,
        &below,
        Point::new(col.text_x(), y),
        Align::Left,
        fg,
    );
}
