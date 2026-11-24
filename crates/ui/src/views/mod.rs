//! One module per screen. Each exposes `regions` (what can change, with a
//! key describing its current content) and `paint` (draws the whole screen;
//! the caller clips it to the region being refreshed).

use embedded_graphics::{prelude::*, primitives::Rectangle};

use crate::{
    gfx::{self, Canvas},
    model::ViewId,
    theme::Theme,
};

pub mod calendar;
pub mod custom;
pub mod forecast;
pub mod home;
pub mod icons;
pub mod menu;
pub mod setup;

/// The size these views are laid out for: the landscape TFT.
pub const W: i32 = 480;
pub const H: i32 = 320;

/// The whole screen of this layout.
#[must_use]
pub const fn area() -> Rectangle {
    gfx::rect(0, 0, W as u32, H as u32)
}

pub struct Region {
    pub rect: Rectangle,
    pub key: u64,
}

pub(crate) fn region(rect: Rectangle, parts: &[i64]) -> Region {
    Region {
        rect,
        key: gfx::key(parts),
    }
}

const NAV_Y: i32 = 312;
const NAV_SPACING: i32 = 14;

/// Most dots there can be: the built-in views and every custom screen.
const NAV_MAX: i32 = (ViewId::BUILT_IN.len() + crate::screen::MAX_SCREENS) as i32;

const fn nav_left(n: usize) -> i32 {
    240 - NAV_SPACING * (n as i32 - 1) / 2
}

/// The dots at the bottom: `n` views, the `i`-th active. The region spans
/// the widest possible row, so dots of removed screens get cleared.
pub(crate) fn nav_region(i: usize, n: usize) -> Region {
    let w = NAV_SPACING * NAV_MAX;
    region(
        gfx::rect(240 - w / 2, NAV_Y - 5, w as u32, 10),
        &[i as i64, n as i64],
    )
}

pub(crate) fn paint_nav(c: &mut Canvas<'_>, i: usize, n: usize, theme: &Theme) {
    for k in 0..n {
        let center = Point::new(nav_left(n) + NAV_SPACING * k as i32, NAV_Y);
        let color = if k == i { theme.fg } else { theme.rule };
        gfx::circle(c, center, 7, color);
    }
}

pub(crate) fn contains(r: &Rectangle, p: Point) -> bool {
    r.contains(p)
}
