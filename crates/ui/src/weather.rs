//! Open-Meteo request and response handling. The platform does the actual
//! HTTP request; this module only builds the URL and parses the answer.

use serde::{Deserialize, Serialize};

use crate::model::{weekday_of, Settings};

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
    /// Maps a WMO weather interpretation code.
    pub fn from_wmo(code: u32) -> Self {
        match code {
            0 => Condition::Clear,
            1 | 2 => Condition::PartlyCloudy,
            3 => Condition::Cloudy,
            45 | 48 => Condition::Fog,
            51..=57 => Condition::Drizzle,
            61..=67 | 80..=82 => Condition::Rain,
            71..=77 | 85 | 86 => Condition::Snow,
            95..=99 => Condition::Thunder,
            _ => Condition::Cloudy,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Condition::Clear => "Clear",
            Condition::PartlyCloudy => "Partly cloudy",
            Condition::Cloudy => "Cloudy",
            Condition::Fog => "Fog",
            Condition::Drizzle => "Drizzle",
            Condition::Rain => "Rain",
            Condition::Snow => "Snow",
            Condition::Thunder => "Thunderstorm",
        }
    }
}

/// Whole degrees with a degree sign, never `-0°`.
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
    /// Local (hour, minute) of the last successful update, if the clock was set.
    pub updated: Option<(u8, u8)>,
    /// Today's sunrise and sunset as local (hour, minute).
    pub sunrise: Option<(u8, u8)>,
    pub sunset: Option<(u8, u8)>,
    /// The next hours, starting with the current one.
    pub hours: Vec<Hour>,
    /// Today and the following days.
    pub days: Vec<Day>,
}

/// How often the weather is refreshed, and how soon a failed fetch is retried.
pub const REFRESH_MS: u64 = 15 * 60 * 1000;
pub const RETRY_MS: u64 = 60 * 1000;
/// Hours and days asked for; the response is about 1.5 KB.
pub const HOURS: usize = 12;
pub const DAYS: usize = 4;

/// Plain HTTP on purpose: TLS would cost about 40 KB of heap on a board
/// without PSRAM, and the request contains nothing but coordinates.
pub fn url(s: &Settings) -> String {
    format!(
        "http://api.open-meteo.com/v1/forecast?latitude={:.4}&longitude={:.4}\
         &current=temperature_2m,weather_code,is_day\
         &hourly=temperature_2m,precipitation_probability,weather_code,is_day&forecast_hours={HOURS}\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset\
         &timezone=auto&forecast_days={DAYS}",
        s.latitude, s.longitude
    )
}

#[derive(Deserialize)]
struct Response {
    current: Current,
    #[serde(default)]
    hourly: Hourly,
    daily: Daily,
}

// f64 on purpose, see `model::de_f32`.
#[derive(Deserialize)]
struct Current {
    temperature_2m: f64,
    weather_code: u32,
    is_day: u8,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Hourly {
    time: Vec<String>,
    temperature_2m: Vec<Option<f64>>,
    precipitation_probability: Vec<Option<f64>>,
    weather_code: Vec<Option<u32>>,
    is_day: Vec<Option<u8>>,
}

#[derive(Deserialize)]
struct Daily {
    #[serde(default)]
    time: Vec<String>,
    #[serde(default)]
    weather_code: Vec<Option<u32>>,
    temperature_2m_max: Vec<Option<f64>>,
    temperature_2m_min: Vec<Option<f64>>,
    #[serde(default)]
    sunrise: Vec<String>,
    #[serde(default)]
    sunset: Vec<String>,
}

/// `2026-11-21T07:38` -> (7, 38)
fn time_of(iso: &str) -> Option<(u8, u8)> {
    let hh = iso.get(11..13)?.parse().ok()?;
    let mm = iso.get(14..16)?.parse().ok()?;
    Some((hh, mm))
}

/// `2026-11-21` -> weekday, 0 = Monday
fn weekday_of_iso(iso: &str) -> Option<u8> {
    let y = iso.get(0..4)?.parse().ok()?;
    let m = iso.get(5..7)?.parse().ok()?;
    let d = iso.get(8..10)?.parse().ok()?;
    Some(weekday_of(y, m, d))
}

pub fn parse(body: &str) -> Result<Weather, String> {
    let r: Response = serde_json::from_str(body).map_err(|e| format!("weather JSON: {e}"))?;
    let d = &r.daily;
    let (Some(Some(max_c)), Some(Some(min_c))) =
        (d.temperature_2m_max.first(), d.temperature_2m_min.first())
    else {
        return Err("weather JSON: empty daily forecast".into());
    };

    let h = &r.hourly;
    let hours = h
        .time
        .iter()
        .enumerate()
        .take(HOURS)
        .map_while(|(i, time)| {
            Some(Hour {
                hour: time_of(time)?.0,
                temp_c: (*h.temperature_2m.get(i)?)? as f32,
                rain_pct: h
                    .precipitation_probability
                    .get(i)
                    .copied()
                    .flatten()
                    .unwrap_or(0.0)
                    .clamp(0.0, 100.0) as u8,
                condition: Condition::from_wmo(h.weather_code.get(i).copied().flatten()?),
                is_day: h.is_day.get(i).copied().flatten().unwrap_or(1) != 0,
            })
        })
        .collect();
    let days = d
        .time
        .iter()
        .enumerate()
        .take(DAYS)
        .map_while(|(i, time)| {
            Some(Day {
                weekday: weekday_of_iso(time)?,
                condition: Condition::from_wmo(d.weather_code.get(i).copied().flatten()?),
                min_c: (*d.temperature_2m_min.get(i)?)? as f32,
                max_c: (*d.temperature_2m_max.get(i)?)? as f32,
            })
        })
        .collect();

    Ok(Weather {
        temp_c: r.current.temperature_2m as f32,
        min_c: *min_c as f32,
        max_c: *max_c as f32,
        condition: Condition::from_wmo(r.current.weather_code),
        is_day: r.current.is_day != 0,
        updated: None,
        sunrise: d.sunrise.first().and_then(|t| time_of(t)),
        sunset: d.sunset.first().and_then(|t| time_of(t)),
        hours,
        days,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real response from api.open-meteo.com, saved on 2026-11-21.
    const SAMPLE: &str = r#"{"latitude":52.512604,"longitude":13.419517,"generationtime_ms":0.40340423583984375,"utc_offset_seconds":3600,"timezone":"Europe/Berlin","timezone_abbreviation":"GMT+1","elevation":38.0,"current_units":{"time":"iso8601","interval":"seconds","temperature_2m":"°C","weather_code":"wmo code","is_day":""},"current":{"time":"2026-11-21T17:00","interval":900,"temperature_2m":6.4,"weather_code":3,"is_day":0},"hourly_units":{"time":"iso8601","temperature_2m":"°C","precipitation_probability":"%","weather_code":"wmo code","is_day":""},"hourly":{"time":["2026-11-21T17:00","2026-11-21T18:00","2026-11-21T19:00","2026-11-21T20:00","2026-11-21T21:00","2026-11-21T22:00","2026-11-21T23:00","2026-11-22T00:00","2026-11-22T01:00","2026-11-22T02:00","2026-11-22T03:00","2026-11-22T04:00"],"temperature_2m":[6.4,6.2,5.9,5.7,5.5,5.3,5.2,4.9,4.8,4.8,4.7,4.5],"precipitation_probability":[23,40,35,18,13,10,20,33,45,45,28,13],"weather_code":[3,3,80,80,61,3,3,3,61,61,3,2],"is_day":[0,0,0,0,0,0,0,0,0,0,0,0]},"daily_units":{"time":"iso8601","weather_code":"wmo code","temperature_2m_max":"°C","temperature_2m_min":"°C","sunrise":"iso8601","sunset":"iso8601"},"daily":{"time":["2026-11-21","2026-11-22","2026-11-23","2026-11-24"],"weather_code":[80,61,77,3],"temperature_2m_max":[6.7,4.9,3.0,1.7],"temperature_2m_min":[0.6,2.1,-0.6,-1.0],"sunrise":["2026-11-21T07:38","2026-11-22T07:39","2026-11-23T07:41","2026-11-24T07:43"],"sunset":["2026-11-21T16:05","2026-11-22T16:04","2026-11-23T16:03","2026-11-24T16:02"]}}"#;

    /// The shape requested by older firmware: current values and today only.
    const SHORT: &str = r#"{"current":{"time":"2026-11-19T19:00","interval":900,"temperature_2m":9.6,"weather_code":3,"is_day":0},"daily":{"time":["2026-11-19"],"temperature_2m_max":[9.8],"temperature_2m_min":[5.6]}}"#;

    #[test]
    fn parses_a_real_response() {
        let w = parse(SAMPLE).unwrap();
        assert_eq!(w.temp_c, 6.4);
        assert_eq!((w.min_c, w.max_c), (0.6, 6.7));
        assert_eq!(w.condition, Condition::Cloudy);
        assert!(!w.is_day);
        assert_eq!(w.sunrise, Some((7, 38)));
        assert_eq!(w.sunset, Some((16, 5)));

        assert_eq!(w.hours.len(), HOURS);
        let h = w.hours[2];
        assert_eq!((h.hour, h.temp_c, h.rain_pct), (19, 5.9, 35));
        assert_eq!(h.condition, Condition::Rain);
        assert_eq!(w.hours[7].hour, 0, "past midnight");

        assert_eq!(w.days.len(), DAYS);
        assert_eq!(w.days[0].weekday, 5, "Saturday");
        assert_eq!(w.days[2].condition, Condition::Snow);
        assert_eq!((w.days[3].min_c, w.days[3].max_c), (-1.0, 1.7));
    }

    #[test]
    fn parses_a_response_without_forecast() {
        let w = parse(SHORT).unwrap();
        assert_eq!((w.temp_c, w.min_c, w.max_c), (9.6, 5.6, 9.8));
        assert!(w.hours.is_empty());
        assert!(w.days.is_empty(), "no weather codes per day");
        assert_eq!(w.sunrise, None);
    }

    #[test]
    fn missing_values_end_the_forecast_early() {
        let body = SAMPLE.replace("[6.4,6.2,5.9,", "[6.4,6.2,null,");
        let w = parse(&body).unwrap();
        assert_eq!(w.hours.len(), 2);
    }

    #[test]
    fn rejects_garbage_and_error_bodies() {
        assert!(parse("").is_err());
        assert!(parse("<html>502</html>").is_err());
        assert!(parse(r#"{"error":true,"reason":"Latitude must be in range"}"#).is_err());
        let empty_daily = r#"{"current":{"temperature_2m":1.0,"weather_code":0,"is_day":1},"daily":{"temperature_2m_max":[],"temperature_2m_min":[]}}"#;
        assert!(parse(empty_daily).is_err());
    }

    #[test]
    fn wmo_codes() {
        assert_eq!(Condition::from_wmo(0), Condition::Clear);
        assert_eq!(Condition::from_wmo(2), Condition::PartlyCloudy);
        assert_eq!(Condition::from_wmo(48), Condition::Fog);
        assert_eq!(Condition::from_wmo(55), Condition::Drizzle);
        assert_eq!(Condition::from_wmo(81), Condition::Rain);
        assert_eq!(Condition::from_wmo(75), Condition::Snow);
        assert_eq!(Condition::from_wmo(96), Condition::Thunder);
        assert_eq!(Condition::from_wmo(1234), Condition::Cloudy);
    }

    #[test]
    fn url_contains_the_coordinates() {
        let s = Settings {
            latitude: 52.52,
            longitude: -13.405,
            ..Settings::default()
        };
        let u = url(&s);
        assert!(u.starts_with("http://api.open-meteo.com/"));
        assert!(u.contains("latitude=52.5200&longitude=-13.4050"));
        assert!(!u.contains(' '));
    }
}
