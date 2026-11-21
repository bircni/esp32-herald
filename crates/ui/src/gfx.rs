//! Drawing helpers.
//!
//! The board has no PSRAM, so a full 320x480 frame buffer (307 KB) does not
//! fit. Instead every region that needs repainting is rendered into a small
//! reusable pixel buffer, band by band, and each band is pushed to the display
//! in one go. That keeps animations flicker-free with about 32 KB of RAM.

use core::convert::Infallible;

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Arc, Circle, Line, PrimitiveStyle, Rectangle, RoundedRectangle, Triangle},
};
use u8g2_fonts::{
    fonts,
    types::{FontColor, HorizontalAlignment, VerticalPosition},
    FontRenderer,
};

use crate::model::screen;

/// Number of pixels in the band buffer (2 bytes each).
pub const BAND_PIXELS: usize = 16_000;

/// An off-screen band of the display, addressed in screen coordinates.
pub struct Canvas<'a> {
    buf: &'a mut [Rgb565],
    area: Rectangle,
}

impl Dimensions for Canvas<'_> {
    fn bounding_box(&self) -> Rectangle {
        self.area
    }
}

impl DrawTarget for Canvas<'_> {
    type Color = Rgb565;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        let (x0, y0) = (self.area.top_left.x, self.area.top_left.y);
        let (w, h) = (self.area.size.width as i32, self.area.size.height as i32);
        for Pixel(p, color) in pixels {
            let (x, y) = (p.x - x0, p.y - y0);
            if x >= 0 && x < w && y >= 0 && y < h {
                self.buf[(y * w + x) as usize] = color;
            }
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        let r = area.intersection(&self.area);
        if r.is_zero_sized() {
            return Ok(());
        }
        let w = self.area.size.width as usize;
        let x = (r.top_left.x - self.area.top_left.x) as usize;
        let y = (r.top_left.y - self.area.top_left.y) as usize;
        for row in y..y + r.size.height as usize {
            let start = row * w + x;
            self.buf[start..start + r.size.width as usize].fill(color);
        }
        Ok(())
    }
}

/// Renders `rect` by calling `paint` once per band and pushing each band to
/// the display. `paint` draws in screen coordinates and must fill the
/// background itself; everything outside the band is clipped.
pub fn render<D>(
    display: &mut D,
    buf: &mut [Rgb565],
    rect: Rectangle,
    mut paint: impl FnMut(&mut Canvas<'_>),
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let rect = rect.intersection(&screen());
    if rect.is_zero_sized() {
        return Ok(());
    }
    let w = rect.size.width as usize;
    let rows_per_band = (buf.len() / w).max(1) as u32;
    let mut y = rect.top_left.y;
    let mut remaining = rect.size.height;
    while remaining > 0 {
        let h = remaining.min(rows_per_band);
        let area = Rectangle::new(Point::new(rect.top_left.x, y), Size::new(w as u32, h));
        let band = &mut buf[..w * h as usize];
        paint(&mut Canvas { buf: band, area });
        display.fill_contiguous(&area, band.iter().copied())?;
        y += h as i32;
        remaining -= h;
    }
    Ok(())
}

pub const fn rect(x: i32, y: i32, w: u32, h: u32) -> Rectangle {
    Rectangle::new(Point::new(x, y), Size::new(w, h))
}

// The canvas cannot fail, so the helpers below swallow the `Infallible` result.

pub fn fill(c: &mut Canvas<'_>, r: Rectangle, color: Rgb565) {
    let _ = c.fill_solid(&r, color);
}

pub fn round_rect(c: &mut Canvas<'_>, r: Rectangle, radius: u32, color: Rgb565) {
    let radius = radius.min(r.size.width / 2).min(r.size.height / 2);
    let _ = RoundedRectangle::with_equal_corners(r, Size::new(radius, radius))
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(c);
}

pub fn round_rect_outline(
    c: &mut Canvas<'_>,
    r: Rectangle,
    radius: u32,
    stroke: u32,
    color: Rgb565,
) {
    let _ = RoundedRectangle::with_equal_corners(r, Size::new(radius, radius))
        .into_styled(PrimitiveStyle::with_stroke(color, stroke))
        .draw(c);
}

pub fn circle(c: &mut Canvas<'_>, center: Point, diameter: u32, color: Rgb565) {
    let _ = Circle::with_center(center, diameter)
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(c);
}

pub fn ring(c: &mut Canvas<'_>, center: Point, diameter: u32, stroke: u32, color: Rgb565) {
    let _ = Circle::with_center(center, diameter)
        .into_styled(PrimitiveStyle::with_stroke(color, stroke))
        .draw(c);
}

pub fn triangle(c: &mut Canvas<'_>, a: Point, b: Point, d: Point, color: Rgb565) {
    let _ = Triangle::new(a, b, d)
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(c);
}

pub fn line(c: &mut Canvas<'_>, a: Point, b: Point, stroke: u32, color: Rgb565) {
    let _ = Line::new(a, b)
        .into_styled(PrimitiveStyle::with_stroke(color, stroke))
        .draw(c);
}

/// Arc with angles in degrees. 0 points right, positive sweeps follow
/// embedded-graphics' convention.
pub fn arc(
    c: &mut Canvas<'_>,
    center: Point,
    diameter: u32,
    start_deg: f32,
    sweep_deg: f32,
    stroke: u32,
    color: Rgb565,
) {
    let _ = Arc::with_center(center, diameter, start_deg.deg(), sweep_deg.deg())
        .into_styled(PrimitiveStyle::with_stroke(color, stroke))
        .draw(c);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Font {
    Small,
    Medium,
    Bold,
    Large,
    Huge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Draws one line of text. `pos.y` is the vertical centre of the line.
pub fn text(c: &mut Canvas<'_>, font: Font, s: &str, pos: Point, align: Align, color: Rgb565) {
    let renderer = match font {
        Font::Small => FontRenderer::new::<fonts::u8g2_font_helvR12_tf>(),
        Font::Medium => FontRenderer::new::<fonts::u8g2_font_helvR18_tf>(),
        Font::Bold => FontRenderer::new::<fonts::u8g2_font_helvB18_tf>(),
        Font::Large => FontRenderer::new::<fonts::u8g2_font_helvB24_tf>(),
        Font::Huge => FontRenderer::new::<fonts::u8g2_font_logisoso50_tf>(),
    };
    let align = match align {
        Align::Left => HorizontalAlignment::Left,
        Align::Center => HorizontalAlignment::Center,
        Align::Right => HorizontalAlignment::Right,
    };
    // Glyphs missing from the font are skipped by the renderer's error path;
    // there is nothing useful to do about them on a 3.5" screen.
    let _ = renderer.render_aligned(
        s,
        pos,
        VerticalPosition::Center,
        align,
        FontColor::Transparent(color),
        c,
    );
}

/// A labelled button.
pub fn button(c: &mut Canvas<'_>, r: Rectangle, label: &str, bg: Rgb565, fg: Rgb565) {
    round_rect(c, r, 14, bg);
    text(c, Font::Bold, label, r.center(), Align::Center, fg);
}

/// Stable 64-bit key for change detection of a screen region.
pub fn key(parts: &[i64]) -> u64 {
    // FNV-1a
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

pub fn str_key(s: &str) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A display that records every pixel it receives.
    struct Recorder {
        px: Vec<Rgb565>,
        calls: usize,
    }

    impl OriginDimensions for Recorder {
        fn size(&self) -> Size {
            Size::new(320, 480)
        }
    }

    impl DrawTarget for Recorder {
        type Color = Rgb565;
        type Error = Infallible;
        fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
        where
            I: IntoIterator<Item = Pixel<Self::Color>>,
        {
            self.calls += 1;
            for Pixel(p, c) in pixels {
                self.px[(p.y * 320 + p.x) as usize] = c;
            }
            Ok(())
        }
    }

    #[test]
    fn render_splits_into_bands_and_clips() {
        let mut d = Recorder {
            px: vec![Rgb565::BLACK; 320 * 480],
            calls: 0,
        };
        // Tiny buffer: 100 px wide region, 250 px buffer => 2 rows per band.
        let mut buf = vec![Rgb565::BLACK; 250];
        let region = rect(10, 20, 100, 5);
        render(&mut d, &mut buf, region, |c| {
            fill(c, screen(), Rgb565::RED);
            // Partly outside the region: must be clipped, not panic.
            fill(c, rect(100, 22, 50, 50), Rgb565::GREEN);
        })
        .unwrap();
        assert_eq!(d.calls, 3, "5 rows at 2 rows per band");
        let at = |x: i32, y: i32| d.px[(y * 320 + x) as usize];
        assert_eq!(at(10, 20), Rgb565::RED);
        assert_eq!(at(109, 21), Rgb565::RED);
        assert_eq!(at(105, 23), Rgb565::GREEN);
        assert_eq!(at(110, 23), Rgb565::BLACK, "right of the region");
        assert_eq!(at(105, 25), Rgb565::BLACK, "below the region");
        assert_eq!(at(9, 20), Rgb565::BLACK, "left of the region");
    }

    #[test]
    fn render_ignores_offscreen_regions() {
        let mut d = Recorder {
            px: vec![Rgb565::BLACK; 320 * 480],
            calls: 0,
        };
        let mut buf = vec![Rgb565::BLACK; 1000];
        render(&mut d, &mut buf, rect(400, 500, 10, 10), |_| {
            panic!("not painted")
        })
        .unwrap();
        assert_eq!(d.calls, 0);
    }

    #[test]
    fn keys_differ() {
        assert_ne!(key(&[1, 2]), key(&[2, 1]));
        assert_eq!(key(&[7]), key(&[7]));
        assert_ne!(str_key("a"), str_key("b"));
    }
}
