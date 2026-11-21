//! Weather icons, drawn from primitives at any size.

use embedded_graphics::{pixelcolor::Rgb565, prelude::*};

use crate::{
    gfx::{self, Canvas},
    theme::Theme,
    weather::Condition,
};

const SUN: Rgb565 = Rgb565::new(31, 50, 4);
const MOON: Rgb565 = Rgb565::new(27, 54, 30);

/// Lengths of the full-size icon (about 160 px across) times the scale.
struct Scale(f32);

impl Scale {
    fn i(&self, v: i32) -> i32 {
        (v as f32 * self.0).round() as i32
    }

    fn u(&self, v: u32) -> u32 {
        ((v as f32 * self.0).round() as u32).max(1)
    }

    fn p(&self, x: i32, y: i32) -> Point {
        Point::new(self.i(x), self.i(y))
    }
}

fn cloud(c: &mut Canvas<'_>, at: Point, s: &Scale, color: Rgb565) {
    gfx::circle(c, at + s.p(-28, 8), s.u(44), color);
    gfx::circle(c, at + s.p(0, -10), s.u(60), color);
    gfx::circle(c, at + s.p(30, 6), s.u(48), color);
    let base = at + s.p(-50, 6);
    gfx::round_rect(
        c,
        gfx::rect(base.x, base.y, s.u(104), s.u(26)),
        s.u(13),
        color,
    );
}

fn sun_or_moon(c: &mut Canvas<'_>, at: Point, d: u32, is_day: bool, bg: Rgb565) {
    if is_day {
        gfx::circle(c, at, d, SUN);
    } else {
        // A disc with a bite taken out of it.
        gfx::circle(c, at, d, MOON);
        let offset = d as i32 / 4;
        gfx::circle(c, at + Point::new(offset, -offset), d - d / 5, bg);
    }
}

/// Draws the icon for `condition` centred on `at`. `bg` is the colour
/// behind it, used to cut the moon's crescent.
pub fn weather(
    c: &mut Canvas<'_>,
    at: Point,
    scale: f32,
    condition: Condition,
    is_day: bool,
    theme: &Theme,
    bg: Rgb565,
) {
    let s = Scale(scale);
    let cloud_color = theme.dim;
    let stroke = s.u(5).max(2);
    let drops = |c: &mut Canvas<'_>, n: i32, len: i32| {
        for i in 0..n {
            let top = at + s.p(-12 * (n - 1) + 24 * i + 6, 42);
            gfx::line(c, top, top + s.p(-len / 2, len), stroke, theme.info);
        }
    };
    match condition {
        Condition::Clear => {
            sun_or_moon(c, at, s.u(84), is_day, bg);
            if is_day {
                for i in 0..8 {
                    let a = i as f32 * core::f32::consts::FRAC_PI_4;
                    let p = |r: f32| at + s.p((a.cos() * r) as i32, (a.sin() * r) as i32);
                    gfx::line(c, p(54.0), p(70.0), s.u(6).max(2), SUN);
                }
            }
        }
        Condition::PartlyCloudy => {
            sun_or_moon(c, at + s.p(26, -26), s.u(64), is_day, bg);
            cloud(c, at + s.p(-8, 12), &s, cloud_color);
        }
        Condition::Cloudy => cloud(c, at, &s, cloud_color),
        Condition::Fog => {
            for i in 0..4 {
                let w = [110, 86, 110, 70][i as usize];
                let p = at + s.p(-55, -36 + 22 * i);
                gfx::round_rect(c, gfx::rect(p.x, p.y, s.u(w), s.u(9)), s.u(4), cloud_color);
            }
        }
        Condition::Drizzle => {
            cloud(c, at + s.p(0, -14), &s, cloud_color);
            drops(c, 3, 10);
        }
        Condition::Rain => {
            cloud(c, at + s.p(0, -14), &s, cloud_color);
            drops(c, 4, 22);
        }
        Condition::Snow => {
            cloud(c, at + s.p(0, -14), &s, cloud_color);
            for i in 0..4 {
                gfx::circle(
                    c,
                    at + s.p(-36 + 24 * i, 42 + 12 * (i % 2)),
                    s.u(9),
                    theme.fg,
                );
            }
        }
        Condition::Thunder => {
            cloud(c, at + s.p(0, -14), &s, cloud_color);
            let p = at + s.p(0, 22);
            gfx::triangle(c, p + s.p(6, 0), p + s.p(-14, 28), p + s.p(4, 24), SUN);
            gfx::triangle(c, p + s.p(-4, 22), p + s.p(16, 20), p + s.p(-8, 52), SUN);
        }
    }
}
