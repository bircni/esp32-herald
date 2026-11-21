//! The device's HTTP API, independent of any HTTP server. The firmware and
//! the simulator both translate their requests into [`Request`] and send
//! back the returned [`Response`].

use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::{
    app::App,
    command::{self, check_item_id, parse_json, reject, Command, Rejection, Reply, Transport},
    model::{Env, ViewId},
};

/// Bodies larger than this are rejected. Transports should not read more.
pub const MAX_BODY: usize = 4096;
pub const TOKEN_HEADER: &str = "X-Buddy-Token";

pub struct Request<'a> {
    pub method: &'a str,
    /// Path, optionally with a query string (which is ignored).
    pub path: &'a str,
    pub body: &'a [u8],
    /// Value of the `X-Buddy-Token` header, if present.
    pub token: Option<&'a str>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

/// Compares without an early exit so the token cannot be guessed byte by
/// byte from response times.
fn token_matches(expected: &str, given: Option<&str>) -> bool {
    let Some(given) = given else {
        return false;
    };
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    diff == 0
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewRequest {
    view: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WifiRequest {
    ssid: String,
    #[serde(default)]
    password: String,
}

/// Plain HTTP with an optional shared token.
pub struct Http;

impl Transport for Http {
    type Message<'a> = Request<'a>;

    fn decode(&self, app: &App, req: &Request<'_>) -> Result<Command, Rejection> {
        let path = req.path.split('?').next().unwrap_or("");
        if req.body.len() > MAX_BODY {
            return Err(reject(413, "request body too large"));
        }
        match (req.method, path) {
            ("GET", "/" | "/index.html") => return Ok(Command::Page),
            // Read-only and free of secrets, so the page can always show it.
            ("GET", "/api/state") => return Ok(Command::State),
            _ => {}
        }
        if !path.starts_with("/api/") {
            return Err(reject(404, "not found"));
        }
        let token = &app.settings().token;
        if !token.is_empty() && !token_matches(token, req.token) {
            return Err(reject(401, "missing or wrong X-Buddy-Token header"));
        }

        if let Some(id) = path.strip_prefix("/api/items/") {
            return match req.method {
                "PUT" | "POST" => {
                    check_item_id(id)?;
                    Ok(Command::PutItem {
                        id: id.into(),
                        item: parse_json(req.body)?,
                    })
                }
                "DELETE" => Ok(Command::DeleteItem(id.into())),
                _ => Err(reject(405, "method not allowed")),
            };
        }

        match (req.method, path) {
            ("GET", "/api/settings") => Ok(Command::GetSettings),
            ("PUT" | "POST", "/api/settings") => {
                Ok(Command::PatchSettings(parse_json::<Map<String, Value>>(
                    req.body,
                )?))
            }
            ("GET", "/api/items") => Ok(Command::GetItems),
            ("DELETE", "/api/items") => Ok(Command::ClearItems),
            ("POST", "/api/view") => {
                let req: ViewRequest = parse_json(req.body)?;
                ViewId::from_name(&req.view)
                    .map(Command::ShowView)
                    .ok_or_else(|| reject(400, format!("unknown view: {}", req.view)))
            }
            ("POST", "/api/wifi") => {
                let req: WifiRequest = parse_json(req.body)?;
                Ok(Command::SetWifi {
                    ssid: req.ssid,
                    password: req.password,
                })
            }
            (_, "/api/settings" | "/api/items" | "/api/view" | "/api/wifi" | "/api/state") => {
                Err(reject(405, "method not allowed"))
            }
            _ => Err(reject(404, "not found")),
        }
    }
}

/// Handles one HTTP request.
pub fn handle(app: &mut App, env: &Env, req: &Request<'_>) -> Response {
    match command::handle(&Http, app, env, req) {
        Ok(Reply::Html(html)) => Response {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: html.into(),
        },
        Ok(Reply::Json(value)) => Response {
            status: 200,
            content_type: "application/json",
            body: value.to_string(),
        },
        Err(e) => Response {
            status: e.status,
            content_type: "application/json",
            body: json!({ "error": e.message }).to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        feed,
        model::{Calibration, Effect, Led, Settings},
    };

    fn app() -> App {
        App::new(Settings::default(), Some(Calibration::identity()))
    }

    fn env(now_ms: u64) -> Env {
        Env {
            now_ms,
            ..Env::default()
        }
    }

    fn call(app: &mut App, now_ms: u64, method: &str, path: &str, body: &str) -> (u16, Value) {
        call_with(app, now_ms, method, path, body, None)
    }

    fn call_with(
        app: &mut App,
        now_ms: u64,
        method: &str,
        path: &str,
        body: &str,
        token: Option<&str>,
    ) -> (u16, Value) {
        let r = handle(
            app,
            &env(now_ms),
            &Request {
                method,
                path,
                body: body.as_bytes(),
                token,
            },
        );
        let value = serde_json::from_str(&r.body).unwrap_or(Value::Null);
        (r.status, value)
    }

    #[test]
    fn serves_the_page() {
        let mut a = app();
        let r = handle(
            &mut a,
            &env(0),
            &Request {
                method: "GET",
                path: "/",
                body: b"",
                token: None,
            },
        );
        assert_eq!(r.status, 200);
        assert!(r.content_type.starts_with("text/html"));
        assert!(r.body.contains("<title>"));
    }

    #[test]
    fn settings_round_trip_and_partial_update() {
        let mut a = app();
        let (status, got) = call(&mut a, 0, "GET", "/api/settings", "");
        assert_eq!(status, 200);
        assert_eq!(got["brightness"], 80);

        let (status, got) = call(
            &mut a,
            0,
            "PUT",
            "/api/settings",
            r##"{"brightness":40,"theme":"light","accent_color":"#FF8800"}"##,
        );
        assert_eq!(status, 200, "{got}");
        assert_eq!(got["brightness"], 40);
        assert_eq!(got["theme"], "light");
        assert_eq!(
            a.settings().timezone,
            Settings::default().timezone,
            "untouched"
        );
        assert_eq!(a.take_effects(), vec![Effect::SaveSettings]);
    }

    #[test]
    fn invalid_settings_change_nothing() {
        let mut a = app();
        for body in [
            r#"{"brightness":0}"#,
            r#"{"brightness":"bright"}"#,
            r#"{"nope":1}"#,
            r#"{"accent_color":"red"}"#,
            r#"[1,2]"#,
            "not json",
        ] {
            let (status, got) = call(&mut a, 0, "PUT", "/api/settings", body);
            assert_eq!(status, 400, "{body} -> {got}");
            assert!(got["error"].is_string());
        }
        assert_eq!(a.settings(), &Settings::default());
        assert!(a.take_effects().is_empty());
    }

    #[test]
    fn unchanged_settings_are_not_saved_again() {
        let mut a = app();
        let (status, _) = call(&mut a, 0, "PUT", "/api/settings", r#"{"brightness":80}"#);
        assert_eq!(status, 200);
        assert!(a.take_effects().is_empty());
    }

    #[test]
    fn token_protects_everything_but_state_and_page() {
        let mut a = app();
        call(&mut a, 0, "PUT", "/api/settings", r#"{"token":"s3cret"}"#);

        assert_eq!(call(&mut a, 0, "GET", "/api/state", "").0, 200);
        assert_eq!(call(&mut a, 0, "GET", "/", "").0, 200);
        assert_eq!(call(&mut a, 0, "GET", "/api/settings", "").0, 401);
        assert_eq!(
            call(&mut a, 0, "PUT", "/api/items/x", r#"{"text":"x"}"#).0,
            401
        );
        for wrong in ["", "s3cre", "s3cret!", "S3CRET"] {
            let r = call_with(&mut a, 0, "GET", "/api/settings", "", Some(wrong));
            assert_eq!(r.0, 401, "token {wrong:?}");
        }
        let r = call_with(&mut a, 0, "GET", "/api/settings", "", Some("s3cret"));
        assert_eq!(r.0, 200);
        assert!(a.feed().is_empty(), "rejected item was not added");

        let (_, state) = call(&mut a, 0, "GET", "/api/state", "");
        assert_eq!(state["token_required"], true);
        assert!(
            !state.to_string().contains("s3cret"),
            "state leaks no secrets"
        );
    }

    #[test]
    fn view_actions() {
        let mut a = app();
        let (status, v) = call(&mut a, 0, "POST", "/api/view", r#"{"view":"forecast"}"#);
        assert_eq!((status, v["view"].as_str()), (200, Some("forecast")));
        assert_eq!(a.view(), ViewId::Forecast);
        for gone in ["timer", "pomodoro", "clock", "tv"] {
            let body = format!(r#"{{"view":"{gone}"}}"#);
            assert_eq!(call(&mut a, 0, "POST", "/api/view", &body).0, 400, "{gone}");
        }
        assert_eq!(call(&mut a, 0, "POST", "/api/timer", "{}").0, 404);
    }

    #[test]
    fn items_are_added_listed_and_removed() {
        let mut a = app();
        let body = r#"{"text":"Kitchen window open","color":"red","age":300}"#;
        let (status, got) = call(&mut a, 600_000, "PUT", "/api/items/window.kitchen", body);
        assert_eq!(status, 200, "{got}");
        let body = r#"{"text":"Claude is working","color":"blue"}"#;
        assert_eq!(
            call(&mut a, 600_000, "POST", "/api/items/claude", body).0,
            200
        );

        let (_, items) = call(&mut a, 660_000, "GET", "/api/items", "");
        let (_, _) = call(
            &mut a,
            1_000,
            "PUT",
            "/api/items/early",
            r#"{"text":"x","age":3600}"#,
        );
        let (_, early) = call(&mut a, 61_000, "GET", "/api/items", "");
        assert_eq!(early[1]["age_s"], 3660, "ages reach back before boot");
        call(&mut a, 0, "DELETE", "/api/items/early", "");
        assert_eq!(items[0]["id"], "window.kitchen", "red comes first");
        assert_eq!(items[0]["age_s"], 360);
        assert_eq!(items[1]["color"], "blue");
        assert_eq!(items[1]["remaining_s"], Value::Null);
        assert_eq!(a.led(), Led::Red, "red item lights the LED");

        let (_, state) = call(&mut a, 660_000, "GET", "/api/state", "");
        assert_eq!(state["items"].as_array().unwrap().len(), 2);

        assert_eq!(
            call(&mut a, 0, "DELETE", "/api/items/window.kitchen", "").0,
            200
        );
        assert_eq!(
            call(&mut a, 0, "DELETE", "/api/items/window.kitchen", "").0,
            404
        );
        assert_eq!(a.led(), Led::Off, "a blue item leaves the LED off");
        assert_eq!(call(&mut a, 0, "DELETE", "/api/items", "").0, 200);
        assert!(a.feed().is_empty());
    }

    #[test]
    fn item_requests_are_validated() {
        let mut a = app();
        for (path, body) in [
            ("/api/items/has%20space", r#"{"text":"x"}"#),
            ("/api/items/", r#"{"text":"x"}"#),
            ("/api/items/ok", r#"{"text":"  "}"#),
            ("/api/items/ok", r#"{}"#),
            ("/api/items/ok", r#"{"text":"x","color":"purple"}"#),
            ("/api/items/ok", r#"{"text":"x","ttl":0}"#),
            ("/api/items/ok", r#"{"text":"x","bogus":1}"#),
            ("/api/items/ok", r#"{"text":"x","mood":"happy"}"#),
        ] {
            let (status, got) = call(&mut a, 0, "PUT", path, body);
            assert_eq!(status, 400, "{path} {body} -> {got}");
        }
        assert_eq!(call(&mut a, 0, "GET", "/api/items/ok", "").0, 405);
        assert!(a.feed().is_empty());

        let long = "y".repeat(100);
        let body = format!(r#"{{"text":"a\nb{long}","detail":"{long}"}}"#);
        assert_eq!(call(&mut a, 0, "PUT", "/api/items/long", &body).0, 200);
        let item = &a.feed()[0];
        assert_eq!(item.text.chars().count(), feed::TEXT_MAX);
        assert!(item.text.starts_with("aby"));
        assert_eq!(item.detail.chars().count(), feed::DETAIL_MAX);
    }

    #[test]
    fn items_expire_and_can_alert() {
        let mut a = app();
        call(&mut a, 0, "POST", "/api/view", r#"{"view":"calendar"}"#);
        a.tick(&env(301_000), None);
        assert!(a.is_asleep());
        let body = r#"{"text":"Door open","color":"yellow","ttl":10}"#;
        call(&mut a, 301_000, "PUT", "/api/items/door", body);
        assert!(a.is_asleep(), "quiet unless asked");
        let body = r#"{"text":"Door open","color":"yellow","ttl":10,"wake":true,"beep":true}"#;
        call(&mut a, 302_000, "PUT", "/api/items/door", body);
        assert!(!a.is_asleep());
        assert_eq!(a.view(), ViewId::Home);
        assert!(a
            .take_effects()
            .contains(&Effect::Beep(crate::model::Beep::Notice)));

        a.tick(&env(311_999), None);
        assert_eq!(a.feed().len(), 1);
        a.tick(&env(312_000), None);
        assert!(a.feed().is_empty());
    }

    #[test]
    fn the_feed_has_a_limit() {
        let mut a = app();
        for i in 0..feed::MAX_ITEMS {
            let path = format!("/api/items/i{i}");
            assert_eq!(call(&mut a, 0, "PUT", &path, r#"{"text":"x"}"#).0, 200);
        }
        assert_eq!(
            call(&mut a, 0, "PUT", "/api/items/full", r#"{"text":"x"}"#).0,
            409
        );
    }

    #[test]
    fn state_reports_weather_with_one_decimal() {
        let mut a = app();
        let body = r#"{"current":{"temperature_2m":9.8,"weather_code":3,"is_day":0},"daily":{"temperature_2m_max":[10.25],"temperature_2m_min":[-5.6],"sunrise":["2026-11-19T07:41"],"sunset":["2026-11-19T16:12"]}}"#;
        a.set_weather(body, &env(0)).unwrap();
        let (_, state) = call(&mut a, 0, "GET", "/api/state", "");
        let w = &state["weather"];
        assert_eq!(
            w.to_string(),
            r#"{"condition":"cloudy","is_day":false,"max_c":10.3,"min_c":-5.6,"sunrise":"07:41","sunset":"16:12","temp_c":9.8,"updated":null}"#
        );
    }

    #[test]
    fn rotation_settings_round_trip() {
        let mut a = app();
        let body = r#"{"rotate":true,"rotate_secs":30,"rotate_views":["home","forecast"],"rotate_from":"08:00","rotate_until":"18:00"}"#;
        let (status, got) = call(&mut a, 0, "PUT", "/api/settings", body);
        assert_eq!(status, 200, "{got}");
        assert_eq!(got["rotate_views"], json!(["home", "forecast"]));
        let bad = r#"{"rotate_views":["home","timer"]}"#;
        assert_eq!(call(&mut a, 0, "PUT", "/api/settings", bad).0, 400);
    }

    #[test]
    fn routing_errors() {
        let mut a = app();
        assert_eq!(call(&mut a, 0, "GET", "/nope", "").0, 404);
        assert_eq!(call(&mut a, 0, "GET", "/api/nope", "").0, 404);
        assert_eq!(call(&mut a, 0, "DELETE", "/api/settings", "").0, 405);
        assert_eq!(call(&mut a, 0, "GET", "/api/state?x=1", "").0, 200);
        let huge = "x".repeat(MAX_BODY + 1);
        assert_eq!(call(&mut a, 0, "PUT", "/api/items/x", &huge).0, 413);
        assert_eq!(call(&mut a, 0, "POST", "/api/status", "{}").0, 404);
    }
}
