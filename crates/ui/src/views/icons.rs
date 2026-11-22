//! Weather icons and symbols as outlines, the same as in the web app (see
//! [`crate::vector`]).

use embedded_graphics::{pixelcolor::Rgb565, prelude::*};

use crate::{
    gfx::Canvas,
    vector::{self, set},
    weather::Condition,
};

/// Small symbols for feed items and custom screens, named as in messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Symbol {
    Window,
    Door,
    Temperature,
    Humidity,
    Water,
    Power,
    Sun,
    Moon,
    Battery,
    Mail,
    Check,
    Warning,
    Info,
    Home,
    Car,
    Music,
    Clock,
    Bell,
    Wind,
}

impl Symbol {
    const fn markup(self) -> &'static str {
        match self {
            Self::Window => set::WINDOW,
            Self::Door => set::DOOR,
            Self::Temperature => set::TEMPERATURE,
            Self::Humidity => set::HUMIDITY,
            Self::Water => set::WATER,
            Self::Power => set::POWER,
            Self::Sun => set::SUN,
            Self::Moon => set::MOON,
            Self::Battery => set::BATTERY,
            Self::Mail => set::MAIL,
            Self::Check => set::CHECK,
            Self::Warning => set::WARNING,
            Self::Info => set::INFO,
            Self::Home => set::HOME,
            Self::Car => set::CAR,
            Self::Music => set::MUSIC,
            Self::Clock => set::CLOCK,
            Self::Bell => set::BELL,
            Self::Wind => set::WIND,
        }
    }
}

/// Draws `sym` in a `size` x `size` box centred on `at`, with a stroke of
/// `stroke` icon units (24 per icon).
pub fn symbol(c: &mut Canvas<'_>, sym: Symbol, at: Point, size: u32, stroke: f32, color: Rgb565) {
    vector::draw(c, sym.markup(), at, size, stroke, color);
}

/// Draws the icon for `condition` in a `size` x `size` box centred on `at`.
pub fn weather(
    c: &mut Canvas<'_>,
    at: Point,
    size: u32,
    condition: Condition,
    is_day: bool,
    stroke: f32,
    color: Rgb565,
) {
    let sky = if is_day { set::SUN } else { set::MOON };
    let up = format!(r#"<g transform="translate(0 -3)">{}</g>"#, set::CLOUD);
    let markup = match condition {
        Condition::Clear => sky.to_owned(),
        Condition::PartlyCloudy => format!(
            r#"<g transform="translate(-3 -3) scale(.7)">{sky}</g><g transform="translate(3 3) scale(.85)">{}</g>"#,
            set::CLOUD
        ),
        Condition::Cloudy => set::CLOUD.to_owned(),
        Condition::Fog => format!(r#"{up}<path d="M4 20h16M7 23h10"/>"#),
        Condition::Drizzle => format!(r#"{up}<path d="M9 19v1M15 19v1M12 21v1"/>"#),
        Condition::Rain => format!(r#"{up}<path d="m8 19-1 3M12 19l-1 3M16 19l-1 3"/>"#),
        Condition::Snow => format!(r#"{up}<path d="M8 20h.01M12 22h.01M16 20h.01"/>"#),
        Condition::Thunder => format!(r#"{up}<path d="m12 15-2 4h3l-2 4"/>"#),
    };
    vector::draw(c, &markup, at, size, stroke, color);
}
