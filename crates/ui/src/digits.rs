//! Seven-segment digits drawn from rounded bars.

use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};

use crate::gfx::{self, Canvas};

/// Size of one digit and its segments.
#[derive(Clone, Copy, Debug)]
pub struct Size7 {
    pub w: i32,
    pub h: i32,
    /// Segment thickness.
    pub t: i32,
    /// Gap between neighbouring segments.
    pub gap: i32,
}

// Bits: a (top), b (top right), c (bottom right), d (bottom),
// e (bottom left), f (top left), g (middle).
const SEGMENTS: [u8; 10] = [
    0b0111111, 0b0000110, 0b1011011, 0b1001111, 0b1100110, 0b1101101, 0b1111101, 0b0000111,
    0b1111111, 0b1101111,
];

pub fn digit_rect(pos: Point, size: Size7) -> Rectangle {
    gfx::rect(pos.x, pos.y, size.w as u32, size.h as u32)
}

/// Draws one digit with its top-left corner at `pos`. `None` draws a dash.
/// Unlit segments are drawn in `off`, which erases them when it is the
/// background colour.
pub fn draw_digit(
    c: &mut Canvas<'_>,
    pos: Point,
    size: Size7,
    value: Option<u8>,
    on: Rgb565,
    off: Rgb565,
) {
    let bits = match value {
        Some(v) => SEGMENTS[usize::from(v % 10)],
        None => 0b1000000,
    };
    let Size7 { w, h, t, gap } = size;
    let half = (h - t) / 2;
    let horiz = |y: i32| {
        gfx::rect(
            pos.x + t / 2 + gap,
            pos.y + y,
            (w - t - 2 * gap) as u32,
            t as u32,
        )
    };
    let vert = |x: i32, y: i32| {
        gfx::rect(
            pos.x + x,
            pos.y + y + t / 2 + gap,
            t as u32,
            (half - 2 * gap) as u32,
        )
    };
    let bars = [
        horiz(0),
        vert(w - t, 0),
        vert(w - t, half),
        horiz(h - t),
        vert(0, half),
        vert(0, 0),
        horiz(half),
    ];
    for (i, bar) in bars.into_iter().enumerate() {
        let color = if bits & (1 << i) != 0 { on } else { off };
        gfx::round_rect(c, bar, t as u32 / 2, color);
    }
}

/// Two dots between hours and minutes; `x` is their centre, `y` the top of
/// the digits.
pub fn draw_colon(c: &mut Canvas<'_>, x: i32, y: i32, size: Size7, color: Rgb565) {
    let d = size.t as u32;
    gfx::circle(c, Point::new(x, y + size.h / 3), d, color);
    gfx::circle(c, Point::new(x, y + 2 * size.h / 3), d, color);
}

/// Splits a two-digit pair like minutes into its digits.
pub fn two(v: u32) -> [u8; 2] {
    [((v / 10) % 10) as u8, (v % 10) as u8]
}
