//! Outline icons from the same SVG markup the web app uses, so the device
//! and the browser draw identical icons.
//!
//! Supports the subset the icons need:
//! `<path d>` (M L H V A C S Z, absolute and relative), `<rect>` with `rx`,
//! `<circle>`, and `<g transform="translate(..) scale(..)">`, all stroked
//! with round caps and joins on a 24-unit grid.

use embedded_graphics::{pixelcolor::Rgb565, prelude::*};

use crate::gfx::{self, Canvas};

/// A point in icon units (0..24).
type P = (f32, f32);

/// Where the 24-unit grid lands on the screen.
#[derive(Clone, Copy)]
struct Place {
    /// Screen position of icon point (0, 0).
    x: f32,
    y: f32,
    /// Pixels per icon unit.
    scale: f32,
}

impl Place {
    fn point(self, (x, y): P) -> Point {
        Point::new(
            (self.x + x * self.scale).round() as i32,
            (self.y + y * self.scale).round() as i32,
        )
    }

    /// Applies a group's `translate(dx dy) scale(s)`.
    fn then(self, dx: f32, dy: f32, s: f32) -> Self {
        Self {
            x: self.x + dx * self.scale,
            y: self.y + dy * self.scale,
            scale: self.scale * s,
        }
    }
}

/// Draws `markup` in a `size` x `size` box centred on `center`, with a
/// stroke of `stroke` icon units (as in the web app's `stroke-width`).
pub fn draw(
    c: &mut Canvas<'_>,
    markup: &str,
    center: Point,
    size: u32,
    stroke: f32,
    color: Rgb565,
) {
    let scale = size as f32 / 24.0;
    let place = Place {
        x: center.x as f32 - size as f32 / 2.0,
        y: center.y as f32 - size as f32 / 2.0,
        scale,
    };
    let width = ((stroke * scale).round() as u32).max(1);
    let mut stack = vec![place];
    let mut rest = markup;
    while let Some(start) = rest.find('<') {
        rest = &rest[start + 1..];
        let end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[..end];
        rest = &rest[end.min(rest.len())..];
        let here = *stack.last().unwrap_or(&place);
        if let Some(closing) = tag.strip_prefix('/') {
            if closing.trim() == "g" && stack.len() > 1 {
                stack.pop();
            }
            continue;
        }
        let name = tag.split_whitespace().next().unwrap_or("");
        match name {
            "g" => stack.push(group(here, attr(tag, "transform").unwrap_or(""))),
            "path" => {
                for line in path(attr(tag, "d").unwrap_or("")) {
                    polyline(c, &line, here, width, color);
                }
            }
            "rect" => {
                let size = (num_attr(tag, "width"), num_attr(tag, "height"));
                let radius = num_attr(tag, "rx").min(size.0 / 2.0).min(size.1 / 2.0);
                let origin = (num_attr(tag, "x"), num_attr(tag, "y"));
                polyline(c, &rounded_rect(origin, size, radius), here, width, color);
            }
            "circle" => {
                let (cx, cy, r) = (num_attr(tag, "cx"), num_attr(tag, "cy"), num_attr(tag, "r"));
                let at = here.point((cx, cy));
                let d = (2.0 * r * here.scale).round() as u32;
                if d <= width + 1 {
                    // Tiny circles (dots) are drawn filled.
                    gfx::circle(c, at, d.max(width), color);
                } else {
                    gfx::ring(c, at, d, width, color);
                }
            }
            _ => {}
        }
    }
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let len = tag[start..].find('"')?;
    Some(&tag[start..start + len])
}

fn num_attr(tag: &str, name: &str) -> f32 {
    attr(tag, name).and_then(|v| v.parse().ok()).unwrap_or(0.0)
}

fn group(place: Place, transform: &str) -> Place {
    let args = |f: &str| -> Vec<f32> {
        transform
            .find(f)
            .map(|i| &transform[i + f.len()..])
            .and_then(|s| {
                s.strip_prefix('(')
                    .map(|s| &s[..s.find(')').unwrap_or(s.len())])
            })
            .map_or_else(Vec::new, numbers)
    };
    let t = args("translate");
    let s = args("scale");
    place.then(
        t.first().copied().unwrap_or(0.0),
        t.get(1).copied().unwrap_or(0.0),
        s.first().copied().unwrap_or(1.0),
    )
}

/// Numbers in SVG syntax: separated by spaces, commas or signs, also
/// `.5.5` and flags glued together as in arcs (`0 0 1 4 0`).
fn numbers(s: &str) -> Vec<f32> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<f32>| {
        if let Ok(v) = cur.parse() {
            out.push(v);
        }
        cur.clear();
    };
    for ch in s.chars() {
        match ch {
            '0'..='9' => cur.push(ch),
            '.' if cur.contains('.') => {
                flush(&mut cur, &mut out);
                cur.push('.');
            }
            '.' => cur.push('.'),
            '-' => {
                flush(&mut cur, &mut out);
                cur.push('-');
            }
            _ => flush(&mut cur, &mut out),
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// A path as polylines in icon units.
#[expect(clippy::too_many_lines, reason = "one arm per path command")]
fn path(d: &str) -> Vec<Vec<P>> {
    let mut lines: Vec<Vec<P>> = Vec::new();
    let mut cur: P = (0.0, 0.0);
    let mut start: P = (0.0, 0.0);
    // Second control point of the last cubic, for `S`.
    let mut last_ctrl: Option<P> = None;
    let mut chars = d.char_indices().peekable();
    while let Some((i, cmd)) = chars.next() {
        if !cmd.is_ascii_alphabetic() {
            continue;
        }
        let end = d[i + 1..]
            .find(|ch: char| ch.is_ascii_alphabetic())
            .map_or(d.len(), |j| i + 1 + j);
        let args = numbers(&d[i + 1..end]);
        let rel = cmd.is_ascii_lowercase();
        let base = |p: P, cur: P| if rel { (cur.0 + p.0, cur.1 + p.1) } else { p };
        let push = |lines: &mut Vec<Vec<P>>, p: P| {
            if let Some(l) = lines.last_mut() {
                l.push(p);
            }
        };
        match cmd.to_ascii_uppercase() {
            'M' => {
                for (k, pair) in args.as_chunks::<2>().0.iter().enumerate() {
                    let p = base((*pair).into(), cur);
                    if k == 0 {
                        lines.push(vec![p]);
                        start = p;
                    } else {
                        push(&mut lines, p);
                    }
                    cur = p;
                }
                last_ctrl = None;
            }
            'L' => {
                for pair in args.as_chunks::<2>().0 {
                    cur = base((*pair).into(), cur);
                    push(&mut lines, cur);
                }
                last_ctrl = None;
            }
            'H' => {
                for x in &args {
                    cur = (if rel { cur.0 + x } else { *x }, cur.1);
                    push(&mut lines, cur);
                }
                last_ctrl = None;
            }
            'V' => {
                for y in &args {
                    cur = (cur.0, if rel { cur.1 + y } else { *y });
                    push(&mut lines, cur);
                }
                last_ctrl = None;
            }
            'C' | 'S' => {
                let per = if cmd.eq_ignore_ascii_case(&'C') { 6 } else { 4 };
                for a in args.chunks_exact(per) {
                    let (c1, c2, to) = if per == 6 {
                        (
                            base((a[0], a[1]), cur),
                            base((a[2], a[3]), cur),
                            base((a[4], a[5]), cur),
                        )
                    } else {
                        let c1 = last_ctrl.map_or(cur, |c| (2.0 * cur.0 - c.0, 2.0 * cur.1 - c.1));
                        (c1, base((a[0], a[1]), cur), base((a[2], a[3]), cur))
                    };
                    for k in 1..=8 {
                        let t = k as f32 / 8.0;
                        let u = 1.0 - t;
                        let p = (
                            u * u * u * cur.0
                                + 3.0 * u * u * t * c1.0
                                + 3.0 * u * t * t * c2.0
                                + t * t * t * to.0,
                            u * u * u * cur.1
                                + 3.0 * u * u * t * c1.1
                                + 3.0 * u * t * t * c2.1
                                + t * t * t * to.1,
                        );
                        push(&mut lines, p);
                    }
                    last_ctrl = Some(c2);
                    cur = to;
                }
            }
            'A' => {
                for a in args.as_chunks::<7>().0 {
                    let to = base((a[5], a[6]), cur);
                    for p in arc(cur, a[0], a[1], a[3] != 0.0, a[4] != 0.0, to) {
                        push(&mut lines, p);
                    }
                    cur = to;
                }
                last_ctrl = None;
            }
            'Z' => {
                push(&mut lines, start);
                cur = start;
                last_ctrl = None;
            }
            _ => {}
        }
        // Skip to the next command.
        while chars.peek().is_some_and(|(j, _)| *j < end) {
            chars.next();
        }
    }
    lines
}

/// Points along an SVG endpoint arc, ending at `to`.
fn arc(from: P, rx: f32, ry: f32, large: bool, sweep: bool, to: P) -> Vec<P> {
    let (rx, ry) = (rx.abs(), ry.abs());
    if rx < f32::EPSILON || ry < f32::EPSILON || from == to {
        return vec![to];
    }
    // Endpoint to centre parameterisation (SVG spec, rotation 0).
    let (dx, dy) = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
    let lambda = (dx * dx) / (rx * rx) + (dy * dy) / (ry * ry);
    let (rx, ry) = if lambda > 1.0 {
        (rx * lambda.sqrt(), ry * lambda.sqrt())
    } else {
        (rx, ry)
    };
    let num = (rx * rx * ry * ry - rx * rx * dy * dy - ry * ry * dx * dx).max(0.0);
    let den = rx * rx * dy * dy + ry * ry * dx * dx;
    let mut k = if den > 0.0 { (num / den).sqrt() } else { 0.0 };
    if large == sweep {
        k = -k;
    }
    let (cxp, cyp) = (k * rx * dy / ry, -k * ry * dx / rx);
    let center = (
        cxp + f32::midpoint(from.0, to.0),
        cyp + f32::midpoint(from.1, to.1),
    );
    let angle = |x: f32, y: f32| y.atan2(x);
    let a0 = angle((dx - cxp) / rx, (dy - cyp) / ry);
    let a1 = angle((-dx - cxp) / rx, (-dy - cyp) / ry);
    let mut delta = a1 - a0;
    let tau = std::f32::consts::TAU;
    if sweep && delta < 0.0 {
        delta += tau;
    } else if !sweep && delta > 0.0 {
        delta -= tau;
    }
    let steps = ((delta.abs() / tau * 32.0).ceil() as usize).max(2);
    (1..=steps)
        .map(|i| {
            let a = a0 + delta * i as f32 / steps as f32;
            if i == steps {
                to
            } else {
                (center.0 + rx * a.cos(), center.1 + ry * a.sin())
            }
        })
        .collect()
}

fn rounded_rect((left, top): P, (width, height): P, radius: f32) -> Vec<P> {
    let corner = |center: P, from_deg: f32| -> Vec<P> {
        (0..=4)
            .map(|i| {
                let angle = (from_deg + 22.5 * i as f32).to_radians();
                (
                    center.0 + radius * angle.cos(),
                    center.1 + radius * angle.sin(),
                )
            })
            .collect()
    };
    let (right, bottom) = (left + width - radius, top + height - radius);
    let mut pts = Vec::with_capacity(21);
    pts.extend(corner((right, top + radius), 270.0));
    pts.extend(corner((right, bottom), 0.0));
    pts.extend(corner((left + radius, bottom), 90.0));
    pts.extend(corner((left + radius, top + radius), 180.0));
    if let Some(first) = pts.first().copied() {
        pts.push(first);
    }
    pts
}

/// Strokes a polyline with round caps and joins.
fn polyline(c: &mut Canvas<'_>, line: &[P], place: Place, width: u32, color: Rgb565) {
    let points: Vec<Point> = line.iter().map(|p| place.point(*p)).collect();
    for pair in points.windows(2) {
        gfx::line(c, pair[0], pair[1], width, color);
    }
    if width > 2 {
        for p in &points {
            gfx::circle(c, *p, width, color);
        }
    } else if let [only] = points.as_slice() {
        gfx::circle(c, *only, width.max(2), color);
    }
}

/// The icon set, in the same markup as the web app's `IC` table.
pub mod set {
    pub const WINDOW: &str =
        r#"<rect x="4" y="3" width="16" height="18" rx="1"/><path d="M12 3v18M4 12h16"/>"#;
    pub const DOOR: &str =
        r#"<rect x="6" y="3" width="12" height="18" rx="1"/><circle cx="15" cy="12" r="1"/>"#;
    pub const TEMPERATURE: &str =
        r#"<path d="M10 14V5a2 2 0 0 1 4 0v9"/><circle cx="12" cy="17" r="3"/>"#;
    pub const HUMIDITY: &str = r#"<path d="M12 3s6 7 6 11a6 6 0 0 1-12 0c0-4 6-11 6-11z"/>"#;
    pub const POWER: &str = r#"<path d="M13 2 5 14h6l-1 8 8-12h-6z"/>"#;
    pub const SUN: &str = r#"<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2M5 5l1.5 1.5M17.5 17.5 19 19M5 19l1.5-1.5M17.5 6.5 19 5"/>"#;
    pub const MOON: &str = r#"<path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z"/>"#;
    pub const BATTERY: &str =
        r#"<rect x="3" y="7" width="16" height="10" rx="2"/><path d="M21 10v4M6 10v4M9 10v4"/>"#;
    pub const MAIL: &str =
        r#"<rect x="3" y="5" width="18" height="14" rx="2"/><path d="m3 7 9 6 9-6"/>"#;
    pub const CHECK: &str = r#"<path d="m5 12 5 5 9-10"/>"#;
    pub const WARNING: &str = r#"<path d="M12 3 22 20H2z"/><path d="M12 10v4M12 17h.01"/>"#;
    pub const INFO: &str = r#"<circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/>"#;
    pub const HOME: &str = r#"<path d="M3 11 12 3l9 8v10H3z"/><path d="M10 21v-6h4v6"/>"#;
    pub const CAR: &str = r#"<path d="M3 13l2-6h14l2 6v5H3z"/><circle cx="7.5" cy="16" r="1.3"/><circle cx="16.5" cy="16" r="1.3"/>"#;
    pub const MUSIC: &str = r#"<path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/>"#;
    pub const CLOCK: &str = r#"<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>"#;
    pub const BELL: &str =
        r#"<path d="M6 16v-5a6 6 0 0 1 12 0v5l2 2H4z"/><path d="M10 21a2 2 0 0 0 4 0"/>"#;
    pub const WATER: &str =
        r#"<path d="M3 9c3-2 6 2 9 0s6-2 9 0M3 14c3-2 6 2 9 0s6-2 9 0M3 19c3-2 6 2 9 0s6-2 9 0"/>"#;
    pub const WIND: &str = r#"<path d="M3 9h12a3 3 0 1 0-3-3M3 15h16a3 3 0 1 1-3 3"/>"#;
    pub const CLOUD: &str =
        r#"<path d="M7 18h10a4 4 0 0 0 .5-8 5.5 5.5 0 0 0-10.6-1.2A4.6 4.6 0 0 0 7 18z"/>"#;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_in_svg_syntax() {
        assert_eq!(numbers("3-2 6 2"), [3.0, -2.0, 6.0, 2.0]);
        assert_eq!(numbers("0 0 1 .5-8"), [0.0, 0.0, 1.0, 0.5, -8.0]);
        assert_eq!(numbers("1.5.5"), [1.5, 0.5]);
    }

    #[test]
    fn paths_become_polylines() {
        let lines = path("M12 3v18M4 12h16");
        assert_eq!(
            lines,
            vec![
                vec![(12.0, 3.0), (12.0, 21.0)],
                vec![(4.0, 12.0), (20.0, 12.0)]
            ]
        );
        let closed = path("M12 3 22 20H2z");
        assert_eq!(closed[0].last(), Some(&(12.0, 3.0)), "z closes the path");
        let half = path("M10 21a2 2 0 0 0 4 0");
        let end = *half[0].last().unwrap();
        assert!(
            (end.0 - 14.0).abs() < 1e-4 && (end.1 - 21.0).abs() < 1e-4,
            "{end:?}"
        );
        assert!(
            half[0].iter().any(|p| p.1 > 22.5),
            "the arc bulges downwards"
        );
    }

    /// The web app and the device must draw the same icons.
    #[test]
    fn icons_match_the_web_app() {
        let html = include_str!("../web/index.html");
        for (name, markup) in [
            ("window", set::WINDOW),
            ("door", set::DOOR),
            ("temperature", set::TEMPERATURE),
            ("humidity", set::HUMIDITY),
            ("power", set::POWER),
            ("sun", set::SUN),
            ("moon", set::MOON),
            ("battery", set::BATTERY),
            ("mail", set::MAIL),
            ("check", set::CHECK),
            ("warning", set::WARNING),
            ("info", set::INFO),
            ("home", set::HOME),
            ("car", set::CAR),
            ("music", set::MUSIC),
            ("clock", set::CLOCK),
            ("bell", set::BELL),
            ("water", set::WATER),
            ("wind", set::WIND),
        ] {
            assert!(
                html.contains(&format!("{name}:'{markup}'")),
                "{name} differs from the web app"
            );
        }
        assert!(html.contains(&format!("const CLOUD='{}'", set::CLOUD)));
    }
}
