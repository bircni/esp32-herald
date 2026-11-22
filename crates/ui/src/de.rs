//! Lenient parsing for values sent by other machines. Home Assistant
//! templates render numbers as strings (`"since": "1795000000"`), so numbers
//! are accepted either way, and text fields also take numbers.

use std::str::FromStr;

use serde::{Deserialize, Deserializer, de::Error};

#[derive(Deserialize)]
#[serde(untagged)]
enum Raw<T> {
    Number(T),
    Text(String),
}

fn parse<T: FromStr, E: Error>(s: &str) -> Result<T, E> {
    s.trim()
        .parse()
        .map_err(|_| E::custom(format!("not a number: {s:?}")))
}

/// An optional number; an empty string counts as absent.
///
/// # Errors
/// When the value is neither a number nor a string holding one.
pub fn opt_number<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + FromStr,
{
    match Option::<Raw<T>>::deserialize(d)? {
        None => Ok(None),
        Some(Raw::Number(n)) => Ok(Some(n)),
        Some(Raw::Text(s)) if s.trim().is_empty() => Ok(None),
        Some(Raw::Text(s)) => parse(&s).map(Some),
    }
}

/// A required number.
///
/// # Errors
/// When the value is neither a number nor a string holding one.
pub fn number<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + FromStr,
{
    match Raw::<T>::deserialize(d)? {
        Raw::Number(n) => Ok(n),
        Raw::Text(s) => parse(&s),
    }
}

/// A required `f32`. Parsed as `f64` and narrowed: the ESP32 toolchain's
/// LLVM cannot compile serde's `f32` path through untagged enums.
///
/// # Errors
/// When the value is neither a number nor a string holding one.
pub fn number_f32<'de, D>(d: D) -> Result<f32, D::Error>
where
    D: Deserializer<'de>,
{
    number::<D, f64>(d).map(|v| v as f32)
}

/// An optional `f32`; see [`number_f32`].
///
/// # Errors
/// When the value is neither a number nor a string holding one.
pub fn opt_number_f32<'de, D>(d: D) -> Result<Option<f32>, D::Error>
where
    D: Deserializer<'de>,
{
    opt_number::<D, f64>(d).map(|v| v.map(|v| v as f32))
}

/// A list of numbers, each also accepted as a string.
///
/// # Errors
/// When the value is neither a number nor a string holding one.
pub fn numbers<'de, D>(d: D) -> Result<Vec<f32>, D::Error>
where
    D: Deserializer<'de>,
{
    Vec::<Raw<f64>>::deserialize(d)?
        .into_iter()
        .map(|v| match v {
            Raw::Number(n) => Ok(n as f32),
            Raw::Text(s) => parse::<f64, _>(&s).map(|n| n as f32),
        })
        .collect()
}

/// Text that may also arrive as a number (`"value": 21.5`).
///
/// # Errors
/// When the value is neither text, a number nor a boolean.
pub fn text<'de, D>(d: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Any {
        Text(String),
        Int(i64),
        Float(f64),
        Bool(bool),
    }
    Ok(match Any::deserialize(d)? {
        Any::Text(s) => s,
        Any::Int(n) => n.to_string(),
        Any::Float(f) => format_float(f),
        Any::Bool(b) => b.to_string(),
    })
}

/// At most two decimals, without trailing zeros: 21.5, 3, 0.25.
#[must_use]
pub fn format_float(f: f64) -> String {
    let s = format!("{f:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_owned()
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize, Debug, PartialEq)]
    struct T {
        #[serde(default, deserialize_with = "opt_number")]
        a: Option<u32>,
        #[serde(deserialize_with = "number")]
        b: f64,
        #[serde(deserialize_with = "text")]
        c: String,
        #[serde(default, deserialize_with = "numbers")]
        d: Vec<f32>,
    }

    #[test]
    fn numbers_and_text_are_lenient() {
        let t: T = serde_json::from_str(r#"{"a":" 5 ","b":"2.5","c":21.50,"d":[1,"2"]}"#).unwrap();
        assert_eq!(
            t,
            T {
                a: Some(5),
                b: 2.5,
                c: "21.5".into(),
                d: vec![1.0, 2.0]
            }
        );
        let t: T = serde_json::from_str(r#"{"a":"","b":1,"c":"x"}"#).unwrap();
        assert_eq!(t.a, None, "empty string is absent");
        serde_json::from_str::<T>(r#"{"b":"soon","c":"x"}"#).unwrap_err();
        assert_eq!(format_float(-0.001), "0");
        assert_eq!(format_float(3.0), "3");
    }
}
