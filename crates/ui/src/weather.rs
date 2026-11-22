//! Weather as pushed by another machine, e.g. Home Assistant over MQTT
//! (see [`Pushed`]). The device fetches nothing itself.

use serde::{Deserialize, Serialize};

use crate::model::weekday_of;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Condition {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Thunder,
}

impl Condition {
    /// Our own names (as serialised) and Home Assistant's weather states.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "clear" | "sunny" | "clear-night" => Self::Clear,
            "partlycloudy" => Self::PartlyCloudy,
            "cloudy" | "windy" | "windy-variant" | "exceptional" => Self::Cloudy,
            "fog" => Self::Fog,
            "drizzle" => Self::Drizzle,
            "rain" | "rainy" | "pouring" => Self::Rain,
            "snow" | "snowy" | "snowy-rainy" | "hail" => Self::Snow,
            "thunder" | "lightning" | "lightning-rainy" => Self::Thunder,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Clear => "Clear",
            Self::PartlyCloudy => "Partly cloudy",
            Self::Cloudy => "Cloudy",
            Self::Fog => "Fog",
            Self::Drizzle => "Drizzle",
            Self::Rain => "Rain",
            Self::Snow => "Snow",
            Self::Thunder => "Thunderstorm",
        }
    }
}

/// Whole degrees with a degree sign, never `-0°`.
#[must_use]
pub fn deg(celsius: f32) -> String {
    let v = celsius.round() as i32;
    format!("{v}\u{b0}")
}

/// One hour of the forecast.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Hour {
    /// Local hour of day, 0..=23.
    pub hour: u8,
    pub temp_c: f32,
    /// Probability of precipitation in percent.
    pub rain_pct: u8,
    pub condition: Condition,
    pub is_day: bool,
}

/// One day of the forecast.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Day {
    /// 0 = Monday.
    pub weekday: u8,
    pub condition: Condition,
    pub min_c: f32,
    pub max_c: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Weather {
    pub temp_c: f32,
    /// Today's range.
    pub min_c: f32,
    pub max_c: f32,
    pub condition: Condition,
    pub is_day: bool,
    /// Local (hour, minute) of the last update, if the clock was set.
    pub updated: Option<(u8, u8)>,
    /// Today's sunrise and sunset as local (hour, minute).
    pub sunrise: Option<(u8, u8)>,
    pub sunset: Option<(u8, u8)>,
    /// The next hours, starting with the current one.
    pub hours: Vec<Hour>,
    /// Today and the following days.
    pub days: Vec<Day>,
}

/// Weather not updated for this long is dropped rather than shown stale.
pub const STALE_MS: u64 = 3 * 60 * 60 * 1000;
/// Hours and days kept from a message.
pub const HOURS: usize = 12;
pub const DAYS: usize = 4;

/// `2026-11-21` -> weekday, 0 = Monday
fn weekday_of_iso(iso: &str) -> Option<u8> {
    let y = iso.get(0..4)?.parse().ok()?;
    let m = iso.get(5..7)?.parse().ok()?;
    let d = iso.get(8..10)?.parse().ok()?;
    Some(weekday_of(y, m, d))
}

/// Weather sent over MQTT (`<prefix>/weather`) or `PUT /api/weather`, e.g.
/// from Home Assistant. Temperatures in °C, times local.
///
/// ```json
/// {"temp": 4.2, "condition": "cloudy", "min": 1, "max": 6, "is_day": true,
///  "sunrise": "07:38", "sunset": "16:05",
///  "hourly": [{"hour": 17, "temp": 3.9, "rain": 20, "condition": "rainy"}],
///  "daily": [{"date": "2026-11-22", "min": 1, "max": 6, "condition": "cloudy"}]}
/// ```
///
/// Fields the device does not use are ignored, so senders may include
/// whatever their source provides (wind, humidity, a location, ...).
#[derive(Debug, PartialEq, Deserialize)]
pub struct Pushed {
    #[serde(deserialize_with = "crate::de::number")]
    temp: f64,
    condition: String,
    /// Today's range; taken from the first day, else the current temperature.
    #[serde(default, deserialize_with = "crate::de::opt_number")]
    min: Option<f64>,
    #[serde(default, deserialize_with = "crate::de::opt_number")]
    max: Option<f64>,
    #[serde(default = "yes")]
    is_day: bool,
    /// `HH:MM`, or an ISO date-time in local time.
    sunrise: Option<String>,
    sunset: Option<String>,
    #[serde(default)]
    hourly: Vec<PushedHour>,
    #[serde(default)]
    daily: Vec<PushedDay>,
}

#[derive(Debug, PartialEq, Deserialize)]
struct PushedHour {
    #[serde(deserialize_with = "crate::de::number")]
    hour: u8,
    #[serde(deserialize_with = "crate::de::number")]
    temp: f64,
    /// Probability of precipitation in percent.
    #[serde(default, deserialize_with = "crate::de::number")]
    rain: f64,
    condition: String,
    #[serde(default = "yes")]
    is_day: bool,
}

#[derive(Debug, PartialEq, Deserialize)]
struct PushedDay {
    /// `YYYY-MM-DD`, or an ISO date-time.
    date: String,
    #[serde(deserialize_with = "crate::de::number")]
    min: f64,
    #[serde(deserialize_with = "crate::de::number")]
    max: f64,
    condition: String,
}

const fn yes() -> bool {
    true
}

fn condition(name: &str) -> Result<Condition, String> {
    Condition::from_name(name).ok_or_else(|| format!("unknown condition: {name}"))
}

fn temp(v: f64) -> Result<f32, String> {
    if (-90.0..=70.0).contains(&v) {
        Ok(v as f32)
    } else {
        Err(format!("temperature out of range: {v}"))
    }
}

/// `07:38` or `2026-11-21T07:38...` -> (7, 38)
fn clock_of(s: &str) -> Result<(u8, u8), String> {
    let time = if s.len() > 10 { s.get(11..) } else { Some(s) };
    let t = time
        .and_then(|t| crate::model::parse_hhmm(t.get(..5)?))
        .ok_or_else(|| format!("time must be HH:MM: {s}"))?;
    Ok(((t / 60) as u8, (t % 60) as u8))
}

impl Pushed {
    /// Checks the values and converts them.
    ///
    /// # Errors
    /// A message naming the first invalid value.
    pub fn into_weather(self) -> Result<Weather, String> {
        let hours = self
            .hourly
            .iter()
            .take(HOURS)
            .map(|h| {
                if h.hour > 23 {
                    return Err(format!("hour must be 0..=23: {}", h.hour));
                }
                Ok(Hour {
                    hour: h.hour,
                    temp_c: temp(h.temp)?,
                    rain_pct: h.rain.clamp(0.0, 100.0) as u8,
                    condition: condition(&h.condition)?,
                    is_day: h.is_day,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let days = self
            .daily
            .iter()
            .take(DAYS)
            .map(|d| {
                Ok(Day {
                    weekday: weekday_of_iso(&d.date)
                        .ok_or_else(|| format!("date must be YYYY-MM-DD: {}", d.date))?,
                    condition: condition(&d.condition)?,
                    min_c: temp(d.min)?,
                    max_c: temp(d.max)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let temp_c = temp(self.temp)?;
        let first = days.first();
        let min_c = match self.min {
            Some(v) => temp(v)?,
            None => first.map_or(temp_c, |d| d.min_c),
        };
        let max_c = match self.max {
            Some(v) => temp(v)?,
            None => first.map_or(temp_c, |d| d.max_c),
        };
        Ok(Weather {
            temp_c,
            min_c: min_c.min(temp_c),
            max_c: max_c.max(temp_c),
            condition: condition(&self.condition)?,
            is_day: self.is_day && self.condition != "clear-night",
            updated: None,
            sunrise: self.sunrise.as_deref().map(clock_of).transpose()?,
            sunset: self.sunset.as_deref().map(clock_of).transpose()?,
            hours,
            days,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pushed(json: &str) -> Result<Weather, String> {
        serde_json::from_str::<Pushed>(json)
            .map_err(|e| e.to_string())?
            .into_weather()
    }

    #[test]
    fn pushed_weather_minimal_and_full() {
        let w = pushed(r#"{"temp":4.2,"condition":"sunny"}"#).unwrap();
        assert_eq!((w.temp_c, w.min_c, w.max_c), (4.2, 4.2, 4.2));
        assert_eq!(w.condition, Condition::Clear);
        assert!(w.is_day && w.hours.is_empty() && w.days.is_empty());

        let w = pushed(
            r#"{"temp":3,"condition":"clear-night","sunrise":"2026-11-22T07:39:00+01:00","sunset":"16:04",
                "hourly":[{"hour":17,"temp":2.5,"rain":120,"condition":"pouring","is_day":false}],
                "daily":[{"date":"2026-11-22","min":-1,"max":5,"condition":"lightning-rainy"},
                         {"date":"2026-11-23T00:00:00+01:00","min":0,"max":4,"condition":"snowy"}]}"#,
        )
        .unwrap();
        assert!(!w.is_day, "clear-night is night");
        assert_eq!((w.min_c, w.max_c), (-1.0, 5.0), "range from the first day");
        assert_eq!((w.sunrise, w.sunset), (Some((7, 39)), Some((16, 4))));
        assert_eq!(w.hours[0].rain_pct, 100);
        assert_eq!(w.hours[0].condition, Condition::Rain);
        assert_eq!(w.days[0].weekday, 6, "Sunday");
        assert_eq!(w.days[0].condition, Condition::Thunder);
        assert_eq!(w.days[1].condition, Condition::Snow);
    }

    #[test]
    fn unknown_weather_fields_are_ignored() {
        let w = pushed(
            r#"{"temp":4,"condition":"rainy","location":"Berlin","wind":3,
                "hourly":[{"hour":1,"temp":2,"condition":"rainy","humidity":80}]}"#,
        )
        .unwrap();
        assert_eq!((w.temp_c, w.hours.len()), (4.0, 1));
    }

    #[test]
    fn pushed_weather_is_checked() {
        for bad in [
            r#"{"condition":"sunny"}"#,
            r#"{"temp":4,"condition":"tornado"}"#,
            r#"{"temp":400,"condition":"sunny"}"#,
            r#"{"temp":4,"condition":"sunny","sunrise":"7am"}"#,
            r#"{"temp":4,"condition":"sunny","hourly":[{"hour":24,"temp":1,"condition":"sunny"}]}"#,
            r#"{"temp":4,"condition":"sunny","daily":[{"date":"monday","min":1,"max":2,"condition":"sunny"}]}"#,
        ] {
            assert!(pushed(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn home_assistant_conditions() {
        for (name, want) in [
            ("sunny", Condition::Clear),
            ("partlycloudy", Condition::PartlyCloudy),
            ("windy-variant", Condition::Cloudy),
            ("fog", Condition::Fog),
            ("pouring", Condition::Rain),
            ("snowy-rainy", Condition::Snow),
            ("lightning", Condition::Thunder),
            ("drizzle", Condition::Drizzle),
        ] {
            assert_eq!(Condition::from_name(name), Some(want), "{name}");
        }
        assert_eq!(Condition::from_name("Sunny"), None);
    }
}
