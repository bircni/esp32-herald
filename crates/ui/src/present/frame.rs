use embedded_graphics::pixelcolor::{Rgb565, RgbColor};

use crate::{
    app::App,
    gfx::{self, Canvas},
    model::Env,
};

/// A partial refresh leaves a faint trace of what was there before. After
/// this many, the next refresh is a full one (black, white, picture).
pub const FULL_EVERY: u32 = 30;

/// One picture, one bit per pixel (set = black), upright.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    width: i32,
    height: i32,
    bits: Vec<u8>,
}

impl Frame {
    #[must_use]
    pub fn blank(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            bits: vec![0; Self::row_bytes_for(width) * height as usize],
        }
    }

    const fn row_bytes_for(width: i32) -> usize {
        (width as usize).div_ceil(8)
    }

    const fn row_bytes(&self) -> usize {
        Self::row_bytes_for(self.width)
    }

    #[must_use]
    pub const fn width(&self) -> i32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> i32 {
        self.height
    }

    /// Whether the pixel at `x`, `y` is black.
    #[must_use]
    pub fn black(&self, x: i32, y: i32) -> bool {
        if !(0..self.width).contains(&x) || !(0..self.height).contains(&y) {
            return false;
        }
        let i = y as usize * self.row_bytes() + x as usize / 8;
        self.bits[i] & (0x80 >> (x % 8)) != 0
    }

    pub fn set_black(&mut self, x: usize, y: usize) {
        let i = y * self.row_bytes() + x / 8;
        self.bits[i] |= 0x80 >> (x % 8);
    }

    /// The picture as a 1-bit BMP file, for the web app.
    #[must_use]
    pub fn to_bmp(&self) -> Vec<u8> {
        // Rows are padded to 4 bytes and stored bottom-up.
        let src = self.row_bytes();
        let row = src.next_multiple_of(4);
        let h = self.height as usize;
        let offset: u32 = 14 + 40 + 8;
        let size = offset + (row * h) as u32;
        let mut out = Vec::with_capacity(size as usize);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&1u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&[0; 4]); // no compression
        out.extend_from_slice(&((row * h) as u32).to_le_bytes());
        out.extend_from_slice(&2835u32.to_le_bytes()); // 72 dpi
        out.extend_from_slice(&2835u32.to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes()); // colours
        out.extend_from_slice(&2u32.to_le_bytes());
        // Colour 0 white, 1 black: the frame's bits as they are.
        out.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0]);
        for y in (0..h).rev() {
            out.extend_from_slice(&self.bits[y * src..(y + 1) * src]);
            out.resize(out.len() + row - src, 0);
        }
        out
    }
}

impl App {
    /// Remembers what the display shows, for the web app's mirror.
    pub fn set_shown_frame(&mut self, frame: Frame) {
        let rev = self.shown_frame.as_ref().map_or(1, |(rev, _)| rev + 1);
        self.shown_frame = Some((rev, frame));
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

/// How to show a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refresh {
    /// Flashes black and white first; clears all traces.
    Full,
    /// Only changes what differs, without flashing.
    Partial,
}

/// Renders the app as 1-bit frames and decides how each reaches the
/// display.
pub struct FramePresenter {
    band_rows: usize,
    /// Colour drawing buffer, `band_rows` rows of the layout's width.
    buf: Vec<Rgb565>,
    shown: Option<Frame>,
    partials: u32,
    force_full: bool,
    /// How the display is turned now; a change needs a full refresh, as
    /// the upright frame stays the same.
    upside_down: bool,
}

impl FramePresenter {
    /// `band_rows` rows are drawn at a time; the layout's height draws all
    /// in one pass (for the e-paper's 272x792, 430 KB: boards with PSRAM).
    #[must_use]
    pub const fn new(band_rows: usize) -> Self {
        Self {
            band_rows,
            buf: Vec::new(),
            shown: None,
            partials: 0,
            force_full: false,
            upside_down: false,
        }
    }

    /// The next refresh is a full one, e.g. after the display failed.
    pub const fn force_full(&mut self) {
        self.force_full = true;
    }

    /// The picture for the app's current state.
    pub fn render(&mut self, app: &App, env: &Env) -> Frame {
        let layout = app.layout();
        let theme = layout.theme(app);
        let size = layout.area().size;
        let (width, height) = (size.width as usize, size.height as usize);
        let rows = self.band_rows.clamp(1, height);
        self.buf.resize(width * rows, Rgb565::WHITE);
        let mut frame = Frame::blank(width as i32, height as i32);
        let mut top = 0;
        while top < height {
            let band = (height - top).min(rows);
            let area = gfx::rect(0, top as i32, width as u32, band as u32);
            let mut canvas = Canvas::new(&mut self.buf, area);
            layout.paint(app, &mut canvas, env, &theme);
            for (i, px) in self.buf[..width * band].iter().enumerate() {
                let (col, row) = (i % width, top + i / width);
                if dark(*px, col, row) {
                    frame.set_black(col, row);
                }
            }
            top += band;
        }
        frame
    }

    /// The frame to show now and how, or `None` when the display already
    /// shows it. Assumes the refresh then happens; if it fails, call
    /// [`FramePresenter::force_full`].
    pub fn next(&mut self, app: &mut App, env: &Env) -> Option<(Frame, Refresh)> {
        if app.take_refresh() {
            self.force_full = true;
        }
        let upside_down = app.settings().upside_down;
        if upside_down != self.upside_down {
            self.upside_down = upside_down;
            self.force_full = true;
        }
        let frame = self.render(app, env);
        let refresh = self.refresh_for(&frame)?;
        Some((frame, refresh))
    }

    fn refresh_for(&mut self, frame: &Frame) -> Option<Refresh> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Action, Profile, Settings,
        model::{LocalTime, ThemeKind},
        theme::Theme,
    };

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
        App::new(Profile::CROWPANEL_579, settings, None)
    }

    #[test]
    fn renders_in_bands_and_at_once_alike() {
        let a = app();
        let whole = FramePresenter::new(1000).render(&a, &env());
        let banded = FramePresenter::new(50).render(&a, &env());
        assert!(whole == banded, "band rendering changes the picture");
        assert_eq!((whole.width(), whole.height()), (272, 792));
        let clock = (0..272).flat_map(|x| (30..100).map(move |y| (x, y)));
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
        let f = FramePresenter::new(1000).render(&a, &env());
        assert!(f.black(2, 2), "the background is black");
    }

    #[test]
    fn dither_keeps_black_and_white_pure() {
        for (x, y) in [(0, 0), (1, 2), (3, 3)] {
            assert!(dark(Rgb565::BLACK, x, y));
            assert!(!dark(Rgb565::WHITE, x, y));
        }
        let grey = Theme::mono(false).rule;
        let dots = (0..16).filter(|i| dark(grey, i % 4, i / 4)).count();
        assert!((5..=11).contains(&dots), "tracks are a mid grey: {dots}");
    }

    #[test]
    fn bmp_has_the_right_size_and_pixels() {
        let mut f = Frame::blank(272, 792);
        f.set_black(0, 0);
        let bmp = f.to_bmp();
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(bmp.len(), 62 + 36 * 792);
        // The top row is stored last.
        assert_eq!(bmp[62 + 36 * 791], 0x80);
        assert_eq!(bmp[62], 0);
    }

    #[test]
    fn refreshes_go_full_now_and_then() {
        let mut a = app();
        let mut p = FramePresenter::new(1000);
        let mut e = env();
        assert_eq!(p.next(&mut a, &e).map(|f| f.1), Some(Refresh::Full));
        assert!(p.next(&mut a, &e).is_none(), "nothing changed");
        let mut kinds = Vec::new();
        for minute in 0..40 {
            if let Some(l) = e.local.as_mut() {
                l.minute = minute;
            }
            kinds.push(p.next(&mut a, &e).map(|f| f.1));
        }
        let full = kinds.iter().filter(|k| **k == Some(Refresh::Full)).count();
        assert_eq!(full, 1, "one full refresh in 40 partial ones: {kinds:?}");

        a.act(Action::Refresh, &e);
        assert_eq!(p.next(&mut a, &e).map(|f| f.1), Some(Refresh::Full));
        a.settings.upside_down = true;
        assert_eq!(
            p.next(&mut a, &e).map(|f| f.1),
            Some(Refresh::Full),
            "turning the display needs a full refresh"
        );
    }
}
