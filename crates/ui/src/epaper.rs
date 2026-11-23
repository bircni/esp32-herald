//! The tall black-and-white layout for e-paper boards (Elecrow `CrowPanel`
//! 5.79", 792x272 used upright as 272x792).
//!
//! E-paper cannot animate and every refresh takes a moment, so instead of
//! the TFT's views this shows one dashboard with everything stacked: clock,
//! weather, the next hours, the feed and the next days. Custom screens are
//! further pages, one widget per row. The buttons page through them; the
//! settings' rotation does the same on its own.
//!
//! Drawing reuses the TFT's helpers in colour and turns the result into
//! black and white at the end, with greys as a dot pattern. The dark
//! theme turns it into white on black.

use embedded_graphics::{
    pixelcolor::{Rgb565, RgbColor},
    prelude::*,
};

use crate::{
    app::{App, Showing},
    feed::{self, Item},
    gfx::{self, Align, Canvas, Font},
    model::{Env, ItemColor, NetState, ThemeKind, ViewId, weekday_abbr},
    theme::Theme,
    views::{custom, home, icons},
    weather::{Hour, Weather, deg},
};

/// Upright size in pixels.
pub const W: i32 = 272;
pub const H: i32 = 792;
/// The panel's own rows: 792 pixels wide, 8 pixels per byte.
pub const PANEL_ROW_BYTES: usize = 99;
pub const PANEL_ROWS: usize = 272;
pub const PANEL_BYTES: usize = PANEL_ROW_BYTES * PANEL_ROWS;

/// A partial refresh leaves a faint trace of what was there before. After
/// this many, the next refresh is a full one (black, white, picture).
pub const FULL_EVERY: u32 = 30;

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

/// Black and white, with one grey for gauge tracks (drawn as a dot
/// pattern).
const fn theme() -> Theme {
    let grey = Rgb565::new(20, 40, 20);
    Theme {
        bg: Rgb565::WHITE,
        fg: Rgb565::BLACK,
        dim: Rgb565::BLACK,
        panel: Rgb565::WHITE,
        rule: grey,
        accent: Rgb565::BLACK,
        on_accent: Rgb565::WHITE,
        good: Rgb565::BLACK,
        warn: Rgb565::BLACK,
        caution: Rgb565::BLACK,
        info: Rgb565::BLACK,
        grey: Rgb565::BLACK,
    }
}

/// One upright picture, one bit per pixel (set = black).
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    bits: Vec<u8>,
}

const FRAME_ROW_BYTES: usize = W as usize / 8;

impl Frame {
    fn blank() -> Self {
        Self {
            bits: vec![0; FRAME_ROW_BYTES * H as usize],
        }
    }

    /// Whether the pixel at `x`, `y` (upright) is black.
    #[must_use]
    pub fn black(&self, x: i32, y: i32) -> bool {
        if !(0..W).contains(&x) || !(0..H).contains(&y) {
            return false;
        }
        let i = y as usize * FRAME_ROW_BYTES + x as usize / 8;
        self.bits[i] & (0x80 >> (x % 8)) != 0
    }

    fn set_black(&mut self, x: usize, y: usize) {
        self.bits[y * FRAME_ROW_BYTES + x / 8] |= 0x80 >> (x % 8);
    }

    /// The picture in the panel's memory layout: rows of 792 pixels, a set
    /// bit is white, the first pixel of a row in the lowest bit of its
    /// last byte (the panel scans mirrored). `flip` turns the picture
    /// upside down, for a board standing the other way round.
    #[must_use]
    pub fn to_panel(&self, flip: bool) -> Vec<u8> {
        let mut out = vec![0xFF; PANEL_BYTES];
        for y in 0..H as usize {
            for x in 0..W as usize {
                if !self.black(x as i32, y as i32) {
                    continue;
                }
                // Upright x runs along the panel's rows, upright y along
                // its columns.
                let (col, row) = if flip {
                    (H as usize - 1 - y, x)
                } else {
                    (y, W as usize - 1 - x)
                };
                let mirrored = 791 - col;
                out[row * PANEL_ROW_BYTES + mirrored / 8] &= !(0x80 >> (mirrored % 8));
            }
        }
        out
    }
}

impl Frame {
    /// The picture as a 1-bit BMP file, for the web app.
    #[must_use]
    pub fn to_bmp(&self) -> Vec<u8> {
        // Rows are padded to 4 bytes and stored bottom-up.
        let row = FRAME_ROW_BYTES.next_multiple_of(4);
        let offset: u32 = 14 + 40 + 8;
        let size = offset + (row * H as usize) as u32;
        let mut out = Vec::with_capacity(size as usize);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&W.to_le_bytes());
        out.extend_from_slice(&H.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&1u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&[0; 4]); // no compression
        out.extend_from_slice(&((row * H as usize) as u32).to_le_bytes());
        out.extend_from_slice(&2835u32.to_le_bytes()); // 72 dpi
        out.extend_from_slice(&2835u32.to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes()); // colours
        out.extend_from_slice(&2u32.to_le_bytes());
        // Colour 0 white, 1 black: the frame's bits as they are.
        out.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0]);
        for y in (0..H as usize).rev() {
            out.extend_from_slice(&self.bits[y * FRAME_ROW_BYTES..(y + 1) * FRAME_ROW_BYTES]);
            out.resize(out.len() + row - FRAME_ROW_BYTES, 0);
        }
        out
    }
}

impl App {
    /// Remembers what the e-paper panel shows, for the web app's mirror.
    pub fn set_shown_frame(&mut self, frame: Frame) {
        let rev = self.shown_frame.as_ref().map_or(1, |(rev, _)| rev + 1);
        self.shown_frame = Some((rev, frame));
    }
}

/// Renders frames. Holds the colour buffer drawing goes through.
pub struct Renderer {
    buf: Vec<Rgb565>,
}

impl Renderer {
    /// `band_rows` rows are drawn at a time; [`H`] draws everything in one
    /// pass (430 KB, for boards with PSRAM).
    #[must_use]
    pub fn new(band_rows: usize) -> Self {
        Self {
            buf: vec![Rgb565::WHITE; W as usize * band_rows.clamp(1, H as usize)],
        }
    }

    /// The picture for the app's current state.
    pub fn render(&mut self, app: &App, env: &Env) -> Frame {
        let theme = theme();
        let invert = app.settings.theme == ThemeKind::Dark;
        let rows = self.buf.len() / W as usize;
        let mut frame = Frame::blank();
        let mut y0 = 0;
        while y0 < H {
            let h = (H - y0).min(rows as i32);
            let area = gfx::rect(0, y0, W as u32, h as u32);
            let mut c = Canvas::new(&mut self.buf, area);
            gfx::fill(&mut c, area, theme.bg);
            paint(app, &mut c, env, &theme);
            for (i, px) in self.buf[..W as usize * h as usize].iter().enumerate() {
                let (x, y) = (i % W as usize, y0 as usize + i / W as usize);
                if dark(*px, x, y) != invert {
                    frame.set_black(x, y);
                }
            }
            y0 += h;
        }
        frame
    }
}

/// 4x4 ordered dither: whether this pixel of a colour comes out black.
fn dark(px: Rgb565, col: usize, row: usize) -> bool {
    const BAYER: [[u16; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let red = u16::from(px.r()) * 255 / 31;
    let green = u16::from(px.g()) * 255 / 63;
    let blue = u16::from(px.b()) * 255 / 31;
    let luma = (red * 3 + green * 6 + blue) / 10;
    luma * 16 < BAYER[row % 4][col % 4] * 16 * 16 + 128
}

/// Decides how each new frame reaches the panel.
#[derive(Default)]
pub struct Refresher {
    shown: Option<Frame>,
    partials: u32,
    force_full: bool,
}

/// How to show a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refresh {
    /// Flashes black and white first; clears all traces.
    Full,
    /// Only changes what differs, without flashing.
    Partial,
}

impl Refresher {
    /// The next refresh is a full one.
    pub const fn force_full(&mut self) {
        self.force_full = true;
    }

    /// How to show `frame`, or `None` when the panel already shows it.
    /// Assumes the refresh then happens.
    pub fn next(&mut self, frame: &Frame) -> Option<Refresh> {
        if !self.force_full && self.shown.as_ref() == Some(frame) {
            return None;
        }
        // A new page changes most of the screen; partial refreshes leave
        // a visible trace of it.
        let mostly_new = self.shown.as_ref().is_none_or(|old| {
            let changed = old
                .bits
                .iter()
                .zip(&frame.bits)
                .filter(|(a, b)| a != b)
                .count();
            changed * 2 > frame.bits.len()
        });
        let full = self.force_full || mostly_new || self.partials + 1 >= FULL_EVERY;
        self.force_full = false;
        self.shown = Some(frame.clone());
        if full {
            self.partials = 0;
            Some(Refresh::Full)
        } else {
            self.partials += 1;
            Some(Refresh::Partial)
        }
    }
}

// ------------------------------------------------------------------ paint

fn center(c: &mut Canvas<'_>, font: Font, text: &str, y: i32, color: Rgb565) {
    let text = gfx::shorten(font, text, W - 2 * MARGIN);
    gfx::text(c, font, &text, Point::new(W / 2, y), Align::Center, color);
}

fn rule(c: &mut Canvas<'_>, y: i32, t: &Theme) {
    gfx::fill(c, gfx::rect(MARGIN, y, (W - 2 * MARGIN) as u32, 2), t.fg);
}

fn paint(app: &App, c: &mut Canvas<'_>, env: &Env, t: &Theme) {
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
    use crate::{Action, Settings, model::LocalTime};

    fn env() -> Env {
        Env {
            now_ms: 600_000,
            local: Some(LocalTime {
                year: 2026,
                month: 11,
                day: 23,
                weekday: 0,
                hour: 14,
                minute: 32,
                second: 0,
            }),
            ..Env::default()
        }
    }

    fn app() -> App {
        let settings = Settings {
            theme: ThemeKind::Light,
            ..Settings::default()
        };
        App::new(crate::Profile::CROWPANEL_579, settings, None)
    }

    #[test]
    fn renders_in_bands_and_at_once_alike() {
        let a = app();
        let whole = Renderer::new(H as usize).render(&a, &env());
        let banded = Renderer::new(50).render(&a, &env());
        assert!(whole == banded, "band rendering changes the picture");
        let clock = (0..W).flat_map(|x| (30..100).map(move |y| (x, y)));
        assert!(
            clock.filter(|(x, y)| whole.black(*x, *y)).count() > 1000,
            "the clock is drawn"
        );
        assert!(!whole.black(2, 2), "the corner stays white");
    }

    #[test]
    fn dark_theme_is_white_on_black() {
        let mut a = app();
        a.settings.theme = ThemeKind::Dark;
        let f = Renderer::new(H as usize).render(&a, &env());
        assert!(f.black(2, 2), "the background is black");
    }

    #[test]
    fn dither_keeps_black_and_white_pure() {
        for (x, y) in [(0, 0), (1, 2), (3, 3)] {
            assert!(dark(Rgb565::BLACK, x, y));
            assert!(!dark(Rgb565::WHITE, x, y));
        }
        let grey = theme().rule;
        let dots = (0..16).filter(|i| dark(grey, i % 4, i / 4)).count();
        assert!((5..=11).contains(&dots), "tracks are a mid grey: {dots}");
    }

    #[test]
    fn panel_layout_maps_corners() {
        let mut f = Frame::blank();
        f.set_black(0, 0);
        let p = f.to_panel(false);
        // Upright top-left: panel row 271, column 0, stored mirrored in the
        // last byte's lowest bit.
        assert_eq!(p[271 * PANEL_ROW_BYTES + 98], !0x01);
        assert_eq!(p.iter().filter(|b| **b != 0xFF).count(), 1);
        let p = f.to_panel(true);
        assert_eq!(p[0], 0x7F, "flipped: panel row 0, column 791");
    }

    #[test]
    fn bmp_has_the_right_size_and_pixels() {
        let mut f = Frame::blank();
        f.set_black(0, 0);
        let bmp = f.to_bmp();
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(bmp.len(), 62 + 36 * H as usize);
        // The top row is stored last.
        assert_eq!(bmp[62 + 36 * (H as usize - 1)], 0x80);
        assert_eq!(bmp[62], 0);
    }

    #[test]
    fn refresher_goes_full_now_and_then() {
        let a = app();
        let mut r = Renderer::new(H as usize);
        let first = r.render(&a, &env());
        let mut refresher = Refresher::default();
        assert_eq!(refresher.next(&first), Some(Refresh::Full));
        assert_eq!(refresher.next(&first), None, "nothing changed");
        let mut later = env();
        let mut kinds = Vec::new();
        for minute in 0..40 {
            if let Some(l) = later.local.as_mut() {
                l.minute = minute;
            }
            let f = r.render(&a, &later);
            kinds.push(refresher.next(&f));
        }
        let full = kinds.iter().filter(|k| **k == Some(Refresh::Full)).count();
        assert_eq!(full, 1, "one full refresh in 40 partial ones: {kinds:?}");
        refresher.force_full();
        assert_eq!(refresher.next(&first), Some(Refresh::Full));
    }

    #[test]
    fn next_and_previous_page_through_screens() {
        let mut a = app();
        let screen = serde_json::from_str(r#"{"widgets":[{"type":"value","value":"1"}]}"#)
            .unwrap_or_else(|e| panic!("{e}"));
        a.put_screen("energy", screen, 0)
            .unwrap_or_else(|e| panic!("{e}"));
        let e = env();
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
        assert_eq!(a.view(), ViewId::Home, "no menu without touch");
    }
}
