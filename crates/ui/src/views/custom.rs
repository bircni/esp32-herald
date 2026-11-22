//! Custom screens: an optional title and up to six widgets in an automatic
//! grid.
//!
//! Laid out like the web app's mirror (`drawScreen` and `widget` in
//! `web/index.html`), so a screen looks the same in the builder and on the
//! device. Each widget is a region of its own, keyed by the screen's content
//! plus whatever changes on the device (countdowns, the clock, the weather).

use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};

use super::{Region, icons, region};
use crate::{
    app::App,
    color::Color,
    de::format_float,
    gfx::{self, Align, Canvas, Font},
    model::Env,
    screen::{ChartStyle, Entry, GaugeStyle, Row, TextSize, Widget, Zone},
    theme::Theme,
    weather::deg,
};

const TITLE_H: i32 = 42;
/// Content ends above the navigation dots.
const BOTTOM: i32 = 300;

const fn title_rect() -> Rectangle {
    gfx::rect(0, 0, 480, TITLE_H as u32)
}

/// Cells for `n` widgets as fractions of the area: 1 full, 2 side by side,
/// 3 in a row, 4 as 2x2, 5-6 as 3x2; then inset like the web app.
fn cells(n: usize, titled: bool) -> Vec<Rectangle> {
    let top = if titled { TITLE_H } else { 8 };
    let height = (BOTTOM - top) as f32;
    let (cols, rows) = match n {
        0 | 1 => (1, 1),
        2 => (2, 1),
        3 => (3, 1),
        4 => (2, 2),
        _ => (3, 2),
    };
    (0..n)
        .map(|i| {
            let (col, row) = (i % cols, i / cols);
            let (col_w, row_h) = (480.0 / cols as f32, height / rows as f32);
            gfx::rect(
                (col as f32 * col_w).round() as i32 + 6,
                top + (row as f32 * row_h).round() as i32 + 4,
                col_w.round() as u32 - 12,
                row_h.round() as u32 - 8,
            )
        })
        .collect()
}

/// The device font closest to a web font size in CSS pixels.
fn font_for(px: f32) -> Font {
    if px >= 56.0 {
        Font::Huge
    } else if px >= 33.0 {
        Font::Temp
    } else if px >= 21.0 {
        Font::Big
    } else if px >= 16.5 {
        Font::Medium
    } else if px >= 14.5 {
        Font::Body
    } else {
        Font::Small
    }
}

/// [`font_for`], stepping down until `text` fits `width`.
fn fit(px: f32, text: &str, width: i32) -> Font {
    const STEPS: [Font; 6] = [
        Font::Huge,
        Font::Temp,
        Font::Big,
        Font::Medium,
        Font::Body,
        Font::Small,
    ];
    let first = font_for(px);
    let from = STEPS.iter().position(|f| *f == first).unwrap_or(0);
    gfx::fitting_font(&STEPS[from..], text, width)
}

/// What changes on the device without a new message.
fn live_key(app: &App, w: &Widget, env: &Env) -> i64 {
    match w {
        Widget::Countdown { until, .. } => match env.unix_s {
            // Seconds in the last hour, minutes before.
            Some(now) if until - now < 3600 => (until - now).max(0),
            Some(now) => (until - now) / 60 + 1_000_000,
            None => -1,
        },
        Widget::Clock => env
            .local
            .map_or(-1, |t| i64::from(t.hour) * 60 + i64::from(t.minute)),
        Widget::Weather => app.weather.as_ref().map_or(-1, |w| {
            ((w.temp_c.round() as i64) << 8) | (w.condition as i64 * 2 + i64::from(w.is_day))
        }),
        Widget::Value { .. }
        | Widget::Gauge { .. }
        | Widget::Chart { .. }
        | Widget::List { .. }
        | Widget::Grid { .. }
        | Widget::Progress { .. }
        | Widget::Text { .. }
        | Widget::Icon { .. } => 0,
    }
}

pub(crate) fn regions(app: &App, slot: u8, env: &Env) -> Vec<Region> {
    let Some(entry) = app.screens.get(slot) else {
        return vec![region(crate::model::screen(), &[-1])];
    };
    let titled = !entry.screen.title.is_empty();
    let theme_key = app.theme().key();
    let mut regions = vec![region(
        title_rect(),
        &[entry.key, i64::from(titled), theme_key],
    )];
    for (i, (w, r)) in entry
        .screen
        .widgets
        .iter()
        .zip(cells(entry.screen.widgets.len(), titled))
        .enumerate()
    {
        regions.push(region(
            r,
            &[entry.key, i as i64, live_key(app, w, env), theme_key],
        ));
    }
    regions
}

pub(crate) fn paint(app: &App, slot: u8, c: &mut Canvas<'_>, env: &Env, theme: &Theme) {
    let Some(entry) = app.screens.get(slot) else {
        return;
    };
    paint_entry(app, entry, c, env, theme);
}

fn paint_entry(app: &App, entry: &Entry, c: &mut Canvas<'_>, env: &Env, theme: &Theme) {
    let screen = &entry.screen;
    let titled = !screen.title.is_empty();
    if titled {
        gfx::text(
            c,
            Font::Bold,
            &screen.title,
            Point::new(16, 22),
            Align::Left,
            theme.fg,
        );
    }
    for (w, r) in screen
        .widgets
        .iter()
        .zip(cells(screen.widgets.len(), titled))
    {
        // Skip widgets outside the band being drawn.
        if r.intersection(&c.bounding_box()).is_zero_sized() {
            continue;
        }
        gfx::round_rect(c, r, 8, theme.panel);
        widget(app, w, c, env, &Cell { r, theme });
    }
}

struct Cell<'a> {
    r: Rectangle,
    theme: &'a Theme,
}

impl Cell<'_> {
    fn color(&self, c: Option<Color>, default: Rgb565) -> Rgb565 {
        c.map_or(default, |c| c.rgb565(self.theme))
    }

    const fn x(&self) -> i32 {
        self.r.top_left.x
    }

    const fn y(&self) -> i32 {
        self.r.top_left.y
    }

    const fn w(&self) -> i32 {
        self.r.size.width as i32
    }

    const fn h(&self) -> i32 {
        self.r.size.height as i32
    }

    const fn cx(&self) -> i32 {
        self.x() + self.w() / 2
    }

    const fn cy(&self) -> i32 {
        self.y() + self.h() / 2
    }

    /// The web app's base size for big values.
    fn big(&self) -> f32 {
        (self.h() as f32 * 0.36)
            .min(self.w() as f32 * 0.28)
            .min(64.0)
    }

    fn label(&self, c: &mut Canvas<'_>, label: &str) {
        if !label.is_empty() {
            let text = gfx::shorten(Font::Small, label, self.w() - 20);
            let at = Point::new(self.x() + 10, self.y() + 16);
            gfx::text(c, Font::Small, &text, at, Align::Left, self.theme.dim);
        }
    }

    /// Text centred at (`cx`, `y`), in the font for `px`, fitted to the cell.
    fn centered(&self, c: &mut Canvas<'_>, text: &str, y: i32, px: f32, color: Rgb565) {
        let font = fit(px, text, self.w() - 12);
        gfx::text(
            c,
            font,
            text,
            Point::new(self.cx(), y),
            Align::Center,
            color,
        );
    }

    /// A horizontal bar with a track, filled to `frac`.
    fn bar(&self, c: &mut Canvas<'_>, y: i32, height: u32, frac: f32, color: Rgb565) {
        let w = (self.w() - 24) as u32;
        let radius = height / 2;
        gfx::round_rect(
            c,
            gfx::rect(self.x() + 12, y, w, height),
            radius,
            self.theme.rule,
        );
        let filled = (w as f32 * frac).round() as u32;
        if filled > 0 {
            let bar = gfx::rect(self.x() + 12, y, filled.max(height), height);
            gfx::round_rect(c, bar, radius, color);
        }
    }
}

fn with_unit(value: &str, unit: &str) -> String {
    if unit.is_empty() {
        value.to_owned()
    } else {
        format!("{value} {unit}")
    }
}

#[expect(clippy::too_many_lines, reason = "one arm per widget type")]
fn widget(app: &App, widget: &Widget, c: &mut Canvas<'_>, env: &Env, cell: &Cell<'_>) {
    let t = cell.theme;
    let (cx, cy) = (cell.cx(), cell.cy());
    match widget {
        Widget::Value {
            label,
            value,
            unit,
            color,
            ..
        } => {
            cell.label(c, label);
            let color = cell.color(*color, t.accent);
            cell.centered(c, &with_unit(value, unit), cy + 6, cell.big(), color);
        }
        Widget::Gauge {
            label,
            value,
            min,
            max,
            unit,
            style,
            color,
            zones,
        } => {
            cell.label(c, label);
            let frac = ((value - min) / (max - min)).clamp(0.0, 1.0);
            let color =
                zone_color(zones, *value, t).unwrap_or_else(|| cell.color(*color, t.accent));
            let number = format_float(f64::from(*value));
            match style {
                GaugeStyle::Bar => {
                    let text = with_unit(&number, unit);
                    cell.centered(c, &text, cy - 4, cell.big().min(40.0), t.fg);
                    cell.bar(c, cy + 24, 10, frac, color);
                }
                GaugeStyle::Ring => {
                    let r = ((cell.w().min(cell.h() - 24)) as f32 * 0.36).max(10.0);
                    let center = Point::new(cx, cy + 10);
                    let d = (2.0 * r).round() as u32;
                    gfx::ring(c, center, d + 9, 9, t.rule);
                    if frac > 0.0 {
                        gfx::arc(c, center, d + 9, -90.0, 360.0 * frac, 9, color);
                    }
                    let text = format!("{number}{unit}");
                    let font = fit((r * 0.6).min(30.0), &text, (2.0 * r) as i32);
                    gfx::text(c, font, &text, center, Align::Center, t.fg);
                }
            }
        }
        Widget::Progress {
            label,
            value,
            max,
            color,
        } => {
            cell.label(c, label);
            let frac = (value / max).clamp(0.0, 1.0);
            let pct = format!("{}%", (frac * 100.0).round() as i32);
            cell.centered(c, &pct, cy - 6, cell.big().min(36.0), t.fg);
            cell.bar(c, cy + 18, 12, frac, cell.color(*color, t.accent));
        }
        Widget::Chart {
            label,
            values,
            min,
            max,
            style,
            color,
            unit,
        } => {
            let heading = if label.is_empty() || unit.is_empty() {
                label.clone()
            } else {
                format!("{label} ({})", unit.trim())
            };
            cell.label(c, &heading);
            let top = cell.y() + if label.is_empty() { 12 } else { 30 };
            let height = cell.h() - if label.is_empty() { 24 } else { 42 };
            let plot = gfx::rect(
                cell.x() + 12,
                top,
                (cell.w() - 24) as u32,
                height.max(1) as u32,
            );
            chart(
                c,
                plot,
                values,
                *min,
                *max,
                *style,
                cell.color(*color, t.accent),
            );
        }
        Widget::List { label, rows } => {
            cell.label(c, label);
            list(c, rows, !label.is_empty(), cell);
        }
        Widget::Grid { tiles } => grid(c, tiles, cell),
        Widget::Countdown {
            label,
            until,
            color,
        } => {
            cell.label(c, label);
            let text = env
                .unix_s
                .map_or_else(|| "--:--".to_owned(), |now| countdown(until - now));
            cell.centered(c, &text, cy + 6, cell.big(), cell.color(*color, t.accent));
        }
        Widget::Text { text, color, size } => {
            let font = match size {
                TextSize::Small => Font::Body,
                TextSize::Medium => Font::Medium,
                TextSize::Large => Font::Big,
            };
            paragraph(c, cell, text, font, cell.color(*color, t.fg));
        }
        Widget::Icon {
            icon,
            label,
            value,
            color,
        } => {
            let size = (cell.h() as f32 * 0.4).min(cell.w() as f32 * 0.4).min(56.0);
            let has_text = !label.is_empty() || !value.is_empty();
            let top = if has_text {
                cell.y() + (cell.h() as f32 * 0.12) as i32
            } else {
                cy - size as i32 / 2
            };
            let at = Point::new(cx, top + size as i32 / 2);
            icons::symbol(c, *icon, at, size as u32, 1.8, cell.color(*color, t.accent));
            if !value.is_empty() {
                let y = cell.y() + (cell.h() as f32 * 0.7) as i32;
                cell.centered(c, value, y, (cell.big() * 0.7).min(30.0), t.fg);
            }
            if !label.is_empty() {
                let y = cell.y() + (cell.h() as f32 * 0.88) as i32;
                cell.centered(c, label, y, 13.0, t.dim);
            }
        }
        Widget::Clock => {
            let text = env.local.map_or_else(
                || "--:--".to_owned(),
                |t| format!("{:02}:{:02}", t.hour, t.minute),
            );
            cell.centered(c, &text, cy, cell.big() * 1.1, t.fg);
        }
        Widget::Weather => weather(app, c, cell),
    }
}

fn zone_color(zones: &[Zone], value: f32, theme: &Theme) -> Option<Rgb565> {
    zones
        .iter()
        .find(|z| value <= z.to)
        .map(|z| z.color.rgb565(theme))
}

/// As in the web app: `2d 4h`, `1:05` (hours and minutes) in the last day,
/// `4:09` (minutes and seconds) in the last hour.
fn countdown(left: i64) -> String {
    let left = left.max(0);
    let (d, h, m, s) = (left / 86_400, left / 3600 % 24, left / 60 % 60, left % 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}:{m:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn chart(
    c: &mut Canvas<'_>,
    area: Rectangle,
    values: &[f32],
    min: Option<f32>,
    max: Option<f32>,
    style: ChartStyle,
    color: Rgb565,
) {
    let lo = min.unwrap_or_else(|| values.iter().copied().fold(f32::MAX, f32::min));
    let hi = max.unwrap_or_else(|| values.iter().copied().fold(f32::MIN, f32::max));
    let span = if hi > lo { hi - lo } else { 1.0 };
    let (x0, y0) = (area.top_left.x, area.top_left.y);
    let (width, height) = (area.size.width as f32, area.size.height as f32);
    let count = values.len();
    let y_of = |v: f32| y0 + (height - ((v - lo) / span).clamp(0.0, 1.0) * height).round() as i32;
    match style {
        ChartStyle::Bar => {
            let slot = width / count as f32;
            for (i, v) in values.iter().enumerate() {
                let top = y_of(*v).min(y0 + height as i32 - 1);
                let x = x0 + (slot * i as f32 + 1.0).round() as i32;
                let w = (slot - 2.0).max(1.0).round() as u32;
                let h = (y0 + height as i32 - top).max(1) as u32;
                gfx::round_rect(c, gfx::rect(x, top, w, h), 1, color);
            }
        }
        ChartStyle::Line => {
            let step = width / (count.max(2) - 1) as f32;
            let points: Vec<Point> = values
                .iter()
                .enumerate()
                .map(|(i, v)| Point::new(x0 + (step * i as f32).round() as i32, y_of(*v)))
                .collect();
            for pair in points.windows(2) {
                gfx::line(c, pair[0], pair[1], 3, color);
            }
        }
    }
}

fn list(c: &mut Canvas<'_>, rows: &[Row], labelled: bool, cell: &Cell<'_>) {
    let top = cell.y() + if labelled { 36 } else { 18 };
    let count = rows.len().max(1) as i32;
    let row_h = ((cell.h() - (top - cell.y()) - 6) / count).min(26);
    let (x0, x1) = (cell.x() + 12, cell.x() + cell.w() - 12);
    for (i, row) in rows.iter().enumerate() {
        let y = top + row_h * i as i32 + row_h / 2;
        let value = with_unit(&row.value, &row.unit);
        let value_w = gfx::text_width(Font::Body, &value);
        let label = gfx::shorten(Font::Body, &row.label, x1 - x0 - value_w - 10);
        gfx::text(
            c,
            Font::Body,
            &label,
            Point::new(x0, y),
            Align::Left,
            cell.theme.fg,
        );
        let color = cell.color(row.color, cell.theme.fg);
        gfx::text(
            c,
            Font::Body,
            &value,
            Point::new(x1, y),
            Align::Right,
            color,
        );
    }
}

fn grid(c: &mut Canvas<'_>, tiles: &[Row], cell: &Cell<'_>) {
    let count = tiles.len().max(1) as i32;
    let cols = if count <= 2 {
        count
    } else if count <= 4 {
        2
    } else {
        3
    };
    let rows = (count + cols - 1) / cols;
    let tw = (cell.w() - 12) as f32 / cols as f32;
    let th = (cell.h() - 12) as f32 / rows as f32;
    for (i, tile) in tiles.iter().enumerate() {
        let (col, row) = ((i as i32 % cols) as f32, (i as i32 / cols) as f32);
        let x = cell.x() as f32 + 6.0 + col * tw + tw / 2.0;
        let y = cell.y() as f32 + 6.0 + row * th;
        let label = gfx::shorten(Font::Small, &tile.label, tw as i32 - 6);
        let at = Point::new(x as i32, (y + th * 0.32) as i32);
        gfx::text(c, Font::Small, &label, at, Align::Center, cell.theme.dim);
        let value = with_unit(&tile.value, &tile.unit);
        let font = fit((th * 0.32).min(26.0), &value, tw as i32 - 6);
        let color = cell.color(tile.color, cell.theme.fg);
        let at = Point::new(x as i32, (y + th * 0.66) as i32);
        gfx::text(c, font, &value, at, Align::Center, color);
    }
}

/// Word-wrapped from the top left, as many lines as fit.
fn paragraph(c: &mut Canvas<'_>, cell: &Cell<'_>, text: &str, font: Font, color: Rgb565) {
    let max_w = cell.w() - 24;
    let line_h = match font {
        Font::Small => 18,
        Font::Body => 20,
        Font::Medium | Font::Bold => 24,
        Font::Big | Font::Large => 34,
        Font::Temp => 44,
        Font::Huge | Font::Day | Font::Clock => 70,
    };
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if gfx::text_width(font, &format!("{line} {word}")) <= max_w => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_owned()),
        }
    }
    let fit = ((cell.h() - 20) / line_h).max(1) as usize;
    for (i, line) in lines.iter().take(fit).enumerate() {
        let y = cell.y() + 10 + line_h * i as i32 + line_h / 2;
        gfx::text(
            c,
            font,
            line,
            Point::new(cell.x() + 12, y),
            Align::Left,
            color,
        );
    }
}

fn weather(app: &App, c: &mut Canvas<'_>, cell: &Cell<'_>) {
    let t = cell.theme;
    let (cx, cy) = (cell.cx(), cell.cy());
    let Some(w) = &app.weather else {
        cell.centered(c, "No weather", cy, 14.0, t.dim);
        return;
    };
    let size = (cell.h() as f32 * 0.38)
        .min(cell.w() as f32 * 0.3)
        .min(52.0) as i32;
    let at = Point::new(cx - size / 2 - 4, cy - 8);
    icons::weather(c, at, size as u32, w.condition, w.is_day, 1.7, t.dim);
    let temp = deg(w.temp_c);
    let font = fit(cell.big().min(40.0), &temp, cell.w() / 2);
    gfx::text(
        c,
        font,
        &temp,
        Point::new(cx + 4, cy - 8),
        Align::Left,
        t.fg,
    );
    cell.centered(c, w.condition.label(), cy + size / 2 + 8, 13.0, t.dim);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_tile_the_screen_without_overlap() {
        for n in 1..=6 {
            for titled in [false, true] {
                let cs = cells(n, titled);
                assert_eq!(cs.len(), n);
                for (i, a) in cs.iter().enumerate() {
                    assert!(
                        a.bottom_right().is_some_and(|p| p.x < 480 && p.y < BOTTOM),
                        "{n} {i}"
                    );
                    for b in &cs[i + 1..] {
                        assert!(a.intersection(b).is_zero_sized(), "cells {n} overlap");
                    }
                }
            }
        }
    }

    #[test]
    fn countdown_like_the_web_app() {
        assert_eq!(countdown(-5), "0:00");
        assert_eq!(countdown(65), "1:05");
        assert_eq!(countdown(3_900), "1:05");
        assert_eq!(countdown(90_000), "1d 1h");
    }

    #[test]
    fn fonts_follow_web_sizes() {
        assert_eq!(font_for(64.0), Font::Huge);
        assert_eq!(font_for(40.0), Font::Temp);
        assert_eq!(font_for(22.0), Font::Big);
        assert_eq!(font_for(13.0), Font::Small);
        assert_eq!(fit(64.0, "a very long value text", 100), Font::Small);
    }
}
