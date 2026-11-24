//! Weather as pushed by another machine, e.g. Home Assistant over MQTT
//! (see [`Pushed`]). The device fetches nothing itself.

use serde::{Deserialize, Serialize};

use crate::model::{LocalTime, weekday_of};

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

    /// A condition from free text such as "Bewölkung zunehmend" or "light
    /// rain showers", by keywords; cloudy when nothing matches.
    #[must_use]
    pub fn guess(text: &str) -> Self {
        const WORDS: [(&[&str], Condition); 8] = [
            (
                &["gewitter", "thunder", "lightning", "storm"],
                Condition::Thunder,
            ),
            (
                &["schnee", "snow", "hagel", "hail", "sleet", "graupel"],
                Condition::Snow,
            ),
            (&["niesel", "drizzle"], Condition::Drizzle),
            (
                &["regen", "rain", "schauer", "shower", "pouring"],
                Condition::Rain,
            ),
            (&["nebel", "fog", "mist", "dunst", "haze"], Condition::Fog),
            (
                &[
                    "heiter",
                    "leicht bew",
                    "teilweise",
                    "wechselnd",
                    "partly",
                    "few clouds",
                ],
                Condition::PartlyCloudy,
            ),
            (
                &["wölk", "wolk", "bedeckt", "cloud", "overcast"],
                Condition::Cloudy,
            ),
            (
                &["sonn", "klar", "clear", "sun", "fair", "dry", "trocken"],
                Condition::Clear,
            ),
        ];
        let text = text.to_lowercase();
        WORDS
            .iter()
            .find(|(words, _)| words.iter().any(|w| text.contains(w)))
            .map_or(Self::Cloudy, |(_, c)| *c)
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
    /// Local date, when the sender gave one.
    #[serde(skip)]
    pub date: Option<(u16, u8, u8)>,
    /// Hours since 1970-01-01 00:00 local time, set when the weather
    /// arrives with the clock set; `None` never counts as past.
    #[serde(skip)]
    pub stamp: Option<i64>,
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

/// Days since 1970-01-01 (proleptic Gregorian calendar).
fn days_from_civil(y: u16, m: u8, d: u8) -> i64 {
    let (m, d) = (i64::from(m.clamp(1, 12)), i64::from(d));
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `LocalTime` as hours since 1970-01-01 00:00.
fn hour_stamp(t: LocalTime) -> i64 {
    days_from_civil(t.year, t.month, t.day) * 24 + i64::from(t.hour)
}

impl Weather {
    /// Dates every hour of the forecast, using `now` for hours sent without
    /// a date (the first one today, the next day after each midnight),
    /// then drops the hours that have passed.
    pub fn date_hours(&mut self, now: Option<&LocalTime>) {
        let Some(now) = now else {
            return;
        };
        let today = days_from_civil(now.year, now.month, now.day);
        let mut day = today;
        let mut last: Option<u8> = None;
        for h in &mut self.hours {
            if last.is_some_and(|l| h.hour < l) {
                day += 1;
            }
            last = Some(h.hour);
            let d = h.date.map_or(day, |(y, m, d)| days_from_civil(y, m, d));
            h.stamp = Some(d * 24 + i64::from(h.hour));
        }
        let first = self.upcoming_from(*now);
        self.hours.drain(..first);
        self.hours.truncate(HOURS);
    }

    fn upcoming_from(&self, now: LocalTime) -> usize {
        let current = hour_stamp(now);
        self.hours
            .iter()
            .position(|h| h.stamp.is_none_or(|s| s >= current))
            .unwrap_or(self.hours.len())
    }

    /// The forecast hours from the current one on.
    #[must_use]
    pub fn upcoming(&self, now: Option<&LocalTime>) -> &[Hour] {
        now.map_or(&self.hours[..], |now| {
            &self.hours[self.upcoming_from(*now)..]
        })
    }
}

/// Weather not updated for this long is dropped rather than shown stale.
pub const STALE_MS: u64 = 3 * 60 * 60 * 1000;
/// Hours and days kept from a message.
pub const HOURS: usize = 12;
/// Hours read from a message: forecasts often start at midnight, and the
/// past ones are dropped once the date is known.
const HOURS_IN: usize = 48;
pub const DAYS: usize = 4;

/// `2026-11-21` (or a date-time starting with it) -> (2026, 11, 21)
fn date_of(iso: &str) -> Option<(u16, u8, u8)> {
    let y = iso.get(0..4)?.parse().ok()?;
    let m = iso.get(5..7)?.parse().ok()?;
    let d = iso.get(8..10)?.parse().ok()?;
    Some((y, m, d))
}

/// `2026-11-21` -> weekday, 0 = Monday
fn weekday_of_iso(iso: &str) -> Option<u8> {
    let (y, m, d) = date_of(iso)?;
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
    /// `YYYY-MM-DD`, or an ISO date-time; optional.
    date: Option<String>,
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

/// Never fails: names outside Home Assistant's list (other integrations
/// send free text, also in German) are guessed from their words, and
/// anything unrecognised counts as cloudy rather than losing the forecast.
#[expect(
    clippy::unnecessary_wraps,
    reason = "same shape as the other field parsers"
)]
fn condition(name: &str) -> Result<Condition, String> {
    Ok(Condition::from_name(name).unwrap_or_else(|| Condition::guess(name)))
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
            .take(HOURS_IN)
            .map(|h| {
                if h.hour > 23 {
                    return Err(format!("hour must be 0..=23: {}", h.hour));
                }
                let date = match &h.date {
                    Some(d) => {
                        Some(date_of(d).ok_or_else(|| format!("date must be YYYY-MM-DD: {d}"))?)
                    }
                    None => None,
                };
                Ok(Hour {
                    hour: h.hour,
                    date,
                    stamp: None,
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
            r#"{"temp":"warm","condition":"sunny"}"#,
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

    #[test]
    fn past_hours_are_skipped() {
        let at = |day, hour| LocalTime {
            year: 2026,
            month: 11,
            day,
            weekday: 0,
            hour,
            minute: 30,
            second: 0,
        };
        // Six-hourly with dates, starting at midnight.
        let mut w = pushed(
            r#"{"temp":3,"condition":"sunny","hourly":[
                {"date":"2026-11-23","hour":1,"temp":1,"condition":"sunny"},
                {"date":"2026-11-23","hour":7,"temp":1,"condition":"sunny"},
                {"date":"2026-11-23","hour":13,"temp":1,"condition":"sunny"},
                {"date":"2026-11-23","hour":19,"temp":1,"condition":"sunny"},
                {"date":"2026-11-24","hour":1,"temp":1,"condition":"sunny"}]}"#,
        )
        .unwrap();
        w.date_hours(Some(&at(23, 13)));
        let hours: Vec<u8> = w.hours.iter().map(|h| h.hour).collect();
        assert_eq!(hours, [13, 19, 1], "the current hour stays");
        let later: Vec<u8> = w
            .upcoming(Some(&at(23, 20)))
            .iter()
            .map(|h| h.hour)
            .collect();
        assert_eq!(later, [1], "hours pass after the weather arrived");
        assert_eq!(w.upcoming(None).len(), 3, "no clock, no filtering");

        // Without dates: the first hour is today, midnight starts the next day.
        let mut w = pushed(
            r#"{"temp":3,"condition":"sunny","hourly":[
                {"hour":22,"temp":1,"condition":"sunny"},
                {"hour":23,"temp":1,"condition":"sunny"},
                {"hour":0,"temp":1,"condition":"sunny"}]}"#,
        )
        .unwrap();
        w.date_hours(Some(&at(23, 22)));
        let late: Vec<u8> = w
            .upcoming(Some(&at(24, 0)))
            .iter()
            .map(|h| h.hour)
            .collect();
        assert_eq!(late, [0]);
    }

    #[test]
    fn free_text_conditions_are_guessed() {
        for (text, want) in [
            ("Bewölkung zunehmend", Condition::Cloudy),
            ("Leicht bewölkt", Condition::PartlyCloudy),
            ("Sunny", Condition::Clear),
            ("dry", Condition::Clear),
            ("Leichter Regenschauer", Condition::Rain),
            ("Schneefall", Condition::Snow),
            ("Gewitter mit Regen", Condition::Thunder),
            ("Nebel", Condition::Fog),
            ("something else", Condition::Cloudy),
        ] {
            assert_eq!(Condition::guess(text), want, "{text}");
        }
    }
}
