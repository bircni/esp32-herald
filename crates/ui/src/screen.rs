//! Custom screens: whole views that other machines define, AWTRIX style.
//!
//! They arrive as retained MQTT messages (`<prefix>/data/screen/<name>`) or over
//! HTTP, live in RAM, and join navigation and rotation next to the built-in
//! views. See [`Screen`] for the format.

use serde::{Deserialize, Serialize};

use crate::{
    color::Color,
    de::{number, number_f32, numbers, opt_number, opt_number_f32, text},
    views::icons::Symbol,
};

/// Screens kept at once.
pub const MAX_SCREENS: usize = 8;
/// Widgets on one screen.
pub const MAX_WIDGETS: usize = 6;
/// Rows of a list, tiles of a grid.
pub const MAX_ROWS: usize = 6;
/// Values of a chart.
pub const MAX_VALUES: usize = 48;
pub const NAME_MAX: usize = 24;
/// Names of built-in views, and the rotation entry for all screens.
const RESERVED: [&str; 4] = ["home", "forecast", "calendar", crate::model::ALL_SCREENS];
const TEXT_MAX: usize = 120;
const LABEL_MAX: usize = 24;

/// One custom screen: an optional title and 1 to 6 widgets, laid out
/// automatically (1 full, 2 side by side, 3 in a row, 4 as 2x2, 5-6 as 3x2).
///
/// ```json
/// {"title": "Energy", "lifetime": 900, "widgets": [
///   {"type": "value", "label": "Solar", "value": 3.2, "unit": "kW", "color": "yellow"},
///   {"type": "gauge", "label": "Battery", "value": 64, "unit": "%"},
///   {"type": "chart", "label": "Today", "values": [0, 0.4, 1.8, 3.2], "unit": "kW"}]}
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Screen {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Seconds without an update after which the screen disappears.
    #[serde(
        default,
        deserialize_with = "opt_number",
        skip_serializing_if = "Option::is_none"
    )]
    pub lifetime: Option<u32>,
    pub widgets: Vec<Widget>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Widget {
    /// A big number or word, with label and unit.
    Value {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        label: String,
        #[serde(deserialize_with = "text")]
        value: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        unit: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<Color>,
        /// A thinner number in the web app; kept for compatibility.
        #[serde(default, skip_serializing_if = "is_false")]
        digits: bool,
    },
    /// A value within a range, as a ring or a bar.
    Gauge {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        label: String,
        #[serde(deserialize_with = "number_f32")]
        value: f32,
        #[serde(default, deserialize_with = "number_f32")]
        min: f32,
        #[serde(default = "hundred", deserialize_with = "number_f32")]
        max: f32,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        unit: String,
        #[serde(default)]
        style: GaugeStyle,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<Color>,
        /// Colour bands: the first zone whose `to` is at least the value wins.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        zones: Vec<Zone>,
    },
    /// A line or bar chart.
    Chart {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        label: String,
        #[serde(deserialize_with = "numbers")]
        values: Vec<f32>,
        #[serde(
            default,
            deserialize_with = "opt_number_f32",
            skip_serializing_if = "Option::is_none"
        )]
        min: Option<f32>,
        #[serde(
            default,
            deserialize_with = "opt_number_f32",
            skip_serializing_if = "Option::is_none"
        )]
        max: Option<f32>,
        #[serde(default)]
        style: ChartStyle,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<Color>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        unit: String,
    },
    /// Label and value rows.
    List {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        label: String,
        rows: Vec<Row>,
    },
    /// Small tiles with label and value.
    Grid { tiles: Vec<Row> },
    /// A bar from 0 to `max`.
    Progress {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        label: String,
        #[serde(deserialize_with = "number_f32")]
        value: f32,
        #[serde(default = "hundred", deserialize_with = "number_f32")]
        max: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<Color>,
    },
    /// Time left until a Unix time; counts down on the device.
    Countdown {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        label: String,
        #[serde(deserialize_with = "number")]
        until: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<Color>,
    },
    /// A wrapped paragraph.
    Text {
        #[serde(deserialize_with = "text")]
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<Color>,
        #[serde(default)]
        size: TextSize,
    },
    /// A symbol with an optional label and value.
    Icon {
        icon: Symbol,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        label: String,
        #[serde(
            default,
            deserialize_with = "text",
            skip_serializing_if = "String::is_empty"
        )]
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<Color>,
    },
    /// The current time.
    Clock,
    /// The current weather.
    Weather,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    #[serde(deserialize_with = "text")]
    pub label: String,
    #[serde(deserialize_with = "text")]
    pub value: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zone {
    #[serde(deserialize_with = "number_f32")]
    pub to: f32,
    pub color: Color,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GaugeStyle {
    #[default]
    Ring,
    Bar,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartStyle {
    #[default]
    Line,
    Bar,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextSize {
    Small,
    #[default]
    Medium,
    Large,
}

const fn hundred() -> f32 {
    100.0
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "signature required by serde"
)]
const fn is_false(b: &bool) -> bool {
    !*b
}

/// 1 to 24 characters of `a-z 0-9 - _`; also the topic level, so no `/`.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    (1..=NAME_MAX).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn check_len(what: &str, s: &str, max: usize) -> Result<(), String> {
    if s.chars().count() > max {
        Err(format!("{what} must be at most {max} characters"))
    } else {
        Ok(())
    }
}

impl Screen {
    /// Checks the limits; parsing already checked types and names.
    ///
    /// # Errors
    /// A message naming the first widget and field that is out of range.
    pub fn validate(&self) -> Result<(), String> {
        check_len("title", &self.title, LABEL_MAX)?;
        if !(1..=MAX_WIDGETS).contains(&self.widgets.len()) {
            return Err(format!("a screen needs 1 to {MAX_WIDGETS} widgets"));
        }
        if self.lifetime == Some(0) {
            return Err("lifetime must be at least 1 second".to_owned());
        }
        for (i, w) in self.widgets.iter().enumerate() {
            w.validate().map_err(|e| format!("widget {}: {e}", i + 1))?;
        }
        Ok(())
    }
}

impl Widget {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Value {
                label, value, unit, ..
            } => {
                check_len("label", label, LABEL_MAX)?;
                check_len("value", value, LABEL_MAX)?;
                check_len("unit", unit, 8)
            }
            Self::Gauge {
                label,
                value,
                min,
                max,
                unit,
                zones,
                ..
            } => {
                check_len("label", label, LABEL_MAX)?;
                check_len("unit", unit, 8)?;
                // Also rejects NaN, which compares as neither.
                if !value.is_finite() || min.partial_cmp(max) != Some(std::cmp::Ordering::Less) {
                    return Err("gauge needs a value and min below max".to_owned());
                }
                if zones.len() > MAX_ROWS {
                    return Err(format!("at most {MAX_ROWS} zones"));
                }
                Ok(())
            }
            Self::Chart {
                label,
                values,
                min,
                max,
                unit,
                ..
            } => {
                check_len("label", label, LABEL_MAX)?;
                check_len("unit", unit, 8)?;
                if values.is_empty() || values.len() > MAX_VALUES {
                    return Err(format!("chart needs 1 to {MAX_VALUES} values"));
                }
                if values.iter().any(|v| !v.is_finite()) {
                    return Err("chart values must be numbers".to_owned());
                }
                if let (Some(lo), Some(hi)) = (min, max)
                    && lo >= hi
                {
                    return Err("chart min must be below max".to_owned());
                }
                Ok(())
            }
            Self::List { label, rows } => {
                check_len("label", label, LABEL_MAX)?;
                check_rows(rows)
            }
            Self::Grid { tiles } => check_rows(tiles),
            Self::Progress {
                label, value, max, ..
            } => {
                check_len("label", label, LABEL_MAX)?;
                if !value.is_finite() || *max <= 0.0 {
                    return Err("progress needs a value and a max above 0".to_owned());
                }
                Ok(())
            }
            Self::Countdown { label, .. } => check_len("label", label, LABEL_MAX),
            Self::Text { text, .. } => check_len("text", text, TEXT_MAX),
            Self::Icon { label, value, .. } => {
                check_len("label", label, LABEL_MAX)?;
                check_len("value", value, LABEL_MAX)
            }
            Self::Clock | Self::Weather => Ok(()),
        }
    }
}

fn check_rows(rows: &[Row]) -> Result<(), String> {
    if rows.is_empty() || rows.len() > MAX_ROWS {
        return Err(format!("needs 1 to {MAX_ROWS} rows"));
    }
    for r in rows {
        check_len("label", &r.label, LABEL_MAX)?;
        check_len("value", &r.value, LABEL_MAX)?;
        check_len("unit", &r.unit, 8)?;
    }
    Ok(())
}

/// A stored screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub screen: Screen,
    /// When it disappears, from `lifetime`.
    pub until_ms: Option<u64>,
    /// Hash of the content, for redraw keys; computed once on arrival.
    pub key: i64,
}

/// The screens in fixed slots, so a view can refer to one by its slot
/// while others come and go.
#[derive(Debug, Default)]
pub struct Screens {
    slots: [Option<Entry>; MAX_SCREENS],
    /// Changes whenever a screen is added, changed or removed.
    rev: u64,
}

impl Screens {
    /// Adds or replaces the screen `name`.
    ///
    /// # Errors
    /// When the name is invalid, the screen breaks a limit or all slots are
    /// taken.
    pub fn put(&mut self, name: &str, screen: Screen, now_ms: u64) -> Result<u8, String> {
        if !valid_name(name) {
            return Err(format!(
                "screen name must be 1..={NAME_MAX} characters of a-z, 0-9, '-', '_'"
            ));
        }
        if RESERVED.contains(&name) {
            return Err(format!(
                "{name:?} is taken by a built-in view; pick another name"
            ));
        }
        screen.validate()?;
        let until_ms = screen.lifetime.map(|s| now_ms + u64::from(s) * 1000);
        let slot = match self.slot(name) {
            Some(slot) => slot,
            None => self
                .slots
                .iter()
                .position(Option::is_none)
                .ok_or_else(|| format!("at most {MAX_SCREENS} screens; remove one first"))?
                as u8,
        };
        let key = crate::gfx::str_key(&serde_json::to_string(&screen).unwrap_or_default());
        let entry = Entry {
            name: name.to_owned(),
            screen,
            until_ms,
            key,
        };
        let cell = &mut self.slots[usize::from(slot)];
        if cell.as_ref() != Some(&entry) {
            *cell = Some(entry);
            self.rev += 1;
        }
        Ok(slot)
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let Some(slot) = self.slot(name) else {
            return false;
        };
        self.slots[usize::from(slot)] = None;
        self.rev += 1;
        true
    }

    /// Drops screens whose lifetime ran out.
    pub fn expire(&mut self, now_ms: u64) {
        for cell in &mut self.slots {
            if cell
                .as_ref()
                .and_then(|e| e.until_ms)
                .is_some_and(|until| now_ms >= until)
            {
                *cell = None;
                self.rev += 1;
            }
        }
    }

    #[must_use]
    pub fn slot(&self, name: &str) -> Option<u8> {
        self.slots
            .iter()
            .position(|e| e.as_ref().is_some_and(|e| e.name == name))
            .map(|i| i as u8)
    }

    #[must_use]
    pub fn get(&self, slot: u8) -> Option<&Entry> {
        self.slots.get(usize::from(slot))?.as_ref()
    }

    /// Slots of all screens, ordered by name.
    #[must_use]
    pub fn ordered(&self) -> Vec<u8> {
        let mut slots: Vec<u8> = (0..MAX_SCREENS as u8)
            .filter(|s| self.get(*s).is_some())
            .collect();
        slots.sort_by(|a, b| {
            let name = |s: &u8| self.get(*s).map(|e| e.name.as_str());
            name(a).cmp(&name(b))
        });
        slots
    }

    #[must_use]
    pub const fn rev(&self) -> u64 {
        self.rev
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Result<Screen, String> {
        let s: Screen = serde_json::from_str(json).map_err(|e| e.to_string())?;
        s.validate()?;
        Ok(s)
    }

    #[test]
    fn every_widget_type_parses_and_round_trips() {
        let json = r##"{"title":"All","widgets":[
            {"type":"value","label":"Solar","value":3.2,"unit":"kW","color":"yellow","digits":true},
            {"type":"gauge","label":"Battery","value":"64","unit":"%","style":"bar","zones":[{"to":20,"color":"red"},{"to":100,"color":"#00FF00"}]},
            {"type":"chart","values":[1,"2",3],"style":"bar","max":5},
            {"type":"list","rows":[{"label":"Kitchen","value":21.5,"unit":"°C"}]},
            {"type":"grid","tiles":[{"label":"CO2","value":812,"color":[255,128,0]}]},
            {"type":"countdown","label":"Train","until":1800000000}
        ]}"##;
        let s = parse(json).unwrap();
        assert_eq!(s.widgets.len(), 6);
        let back: Screen = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(
            back, s,
            "serialising and parsing again gives the same screen"
        );

        for extra in [
            r#"{"type":"progress","value":40}"#,
            r#"{"type":"text","text":"Hello there","size":"large"}"#,
            r#"{"type":"icon","icon":"window","label":"Kitchen","value":"open","color":"red"}"#,
            r#"{"type":"clock"}"#,
            r#"{"type":"weather"}"#,
        ] {
            parse(&format!(r#"{{"widgets":[{extra}]}}"#)).unwrap();
        }
    }

    #[test]
    fn limits_and_mistakes_are_reported() {
        for (bad, needle) in [
            (r#"{"widgets":[]}"#, "1 to 6"),
            (r#"{"widgets":[{"type":"dial"}]}"#, "unknown variant"),
            (r#"{"widgets":[{"type":"value"}]}"#, "missing field"),
            (
                r#"{"widgets":[{"type":"gauge","value":1,"min":5,"max":5}]}"#,
                "widget 1",
            ),
            (r#"{"widgets":[{"type":"chart","values":[]}]}"#, "1 to 48"),
            (r#"{"widgets":[{"type":"list","rows":[]}]}"#, "rows"),
            (
                r#"{"widgets":[{"type":"icon","icon":"unicorn"}]}"#,
                "unknown variant",
            ),
            (r#"{"lifetime":0,"widgets":[{"type":"clock"}]}"#, "lifetime"),
            (r#"{"widgets":[{"type":"clock"}],"wat":1}"#, "unknown field"),
        ] {
            let err = parse(bad).unwrap_err();
            assert!(err.contains(needle), "{bad}: {err}");
        }
    }

    #[test]
    fn store_keeps_slots_and_order() {
        let clock = || parse(r#"{"widgets":[{"type":"clock"}]}"#).unwrap();
        let mut s = Screens::default();
        let zed = s.put("zed", clock(), 0).unwrap();
        let alpha = s.put("alpha", clock(), 0).unwrap();
        assert_eq!(s.ordered(), [alpha, zed], "ordered by name");
        let rev = s.rev();
        assert_eq!(s.put("zed", clock(), 0).unwrap(), zed, "same slot");
        assert_eq!(s.rev(), rev, "unchanged screen keeps the revision");
        s.put("Bad Name", clock(), 0).unwrap_err();
        for i in 0..MAX_SCREENS - 2 {
            s.put(&format!("s{i}"), clock(), 0).unwrap();
        }
        assert!(
            s.put("one-too-many", clock(), 0)
                .unwrap_err()
                .contains("at most")
        );
        for taken in ["home", "forecast", "calendar", "screens"] {
            let err = s.put(taken, clock(), 0).unwrap_err();
            assert!(err.contains("built-in"), "{taken}: {err}");
        }
        assert!(s.remove("alpha"));
        assert!(!s.remove("alpha"));
        assert_eq!(s.slot("zed"), Some(zed), "others keep their slot");
    }

    #[test]
    fn lifetime_removes_stale_screens() {
        let mut s = Screens::default();
        let screen = parse(r#"{"lifetime":10,"widgets":[{"type":"clock"}]}"#).unwrap();
        s.put("temp", screen, 1_000).unwrap();
        s.expire(10_999);
        assert!(s.slot("temp").is_some());
        s.expire(11_000);
        assert!(s.slot("temp").is_none());
    }
}
