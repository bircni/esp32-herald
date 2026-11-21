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
pub mod forecast;
pub mod home;
pub mod icons;
pub mod menu;
pub mod setup;

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
const NAV_SPACING: i32 = 16;

fn nav_left() -> i32 {
    240 - NAV_SPACING * (ViewId::ALL.len() as i32 - 1) / 2
}

/// The dots at the bottom showing which view is active.
pub(crate) fn nav_region(view: ViewId) -> Region {
    let w = NAV_SPACING * ViewId::ALL.len() as i32;
    region(
        gfx::rect(nav_left() - 8, NAV_Y - 5, w as u32, 10),
        &[view.index() as i64],
    )
}

pub(crate) fn paint_nav(c: &mut Canvas<'_>, view: ViewId, theme: &Theme) {
    for (i, v) in ViewId::ALL.into_iter().enumerate() {
        let center = Point::new(nav_left() + NAV_SPACING * i as i32, NAV_Y);
        if v == view {
            gfx::circle(c, center, 8, theme.fg);
        } else {
            gfx::circle(c, center, 6, theme.panel);
        }
    }
}

pub(crate) fn contains(r: &Rectangle, p: Point) -> bool {
    r.contains(p)
}
