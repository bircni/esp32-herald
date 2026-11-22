//! The device's HTTP API, independent of any HTTP server. The firmware and
//! the simulator both translate their requests into [`Request`] and send
//! back the returned [`Response`].

use std::borrow::Cow;

use embedded_graphics::prelude::Point;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    app::App,
    command::{
        self, Command, Rejection, Reply, Transport, check_item_id, parse_item, parse_json, reject,
    },
    gesture::{Dir, Gesture},
    gfx,
    model::{Env, NetState, screen},
};

/// Bodies larger than this are rejected. Transports should not read more,
/// except for firmware updates, which the firmware streams itself.
pub const MAX_BODY: usize = 4096;
pub const TOKEN_HEADER: &str = "X-Herald-Token";
/// Firmware updates; handled by the firmware, which streams the body.
pub const OTA_PATH: &str = "/api/ota";

pub struct Request<'a> {
    pub method: &'a str,
    /// Path, optionally with a query string (which is ignored).
    pub path: &'a str,
    pub body: &'a [u8],
    /// Value of the `X-Herald-Token` header, if present.
    pub token: Option<&'a str>,
    /// Value of the `If-None-Match` header, if present.
    pub if_none_match: Option<&'a str>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    /// Further headers, e.g. `ETag`, `Content-Encoding`, `Location`.
    pub headers: Vec<(&'static str, String)>,
    pub body: Cow<'static, [u8]>,
}

impl Response {
    fn json(status: u16, value: &Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            headers: Vec::new(),
            body: Cow::Owned(value.to_string().into_bytes()),
        }
    }

    fn redirect(to: &str) -> Self {
        Self {
            status: 302,
            content_type: "text/plain",
            headers: vec![("Location", to.to_owned())],
            body: Cow::Borrowed(b""),
        }
    }

    fn not_modified(etag: String) -> Self {
        Self {
            status: 304,
            content_type: "text/plain",
            headers: vec![("ETag", etag)],
            body: Cow::Borrowed(b""),
        }
    }

    /// The body as text, for tests and logs.
    #[must_use]
    pub fn text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
    }

    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Whether a request with `token` may change things: always when no token
/// is set. For routes the platform handles itself, such as firmware updates.
#[must_use]
pub fn authorized(app: &App, token: Option<&str>) -> bool {
    let expected = &app.settings().token;
    expected.is_empty() || token_matches(expected, token)
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

/// `{"x":10,"y":20}` taps, with `"long":true` it is a long press.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TapRequest {
    x: i32,
    y: i32,
    #[serde(default)]
    long: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SwipeRequest {
    dir: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DisplayRequest {
    on: bool,
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
        if !authorized(app, req.token) {
            return Err(reject(401, "missing or wrong X-Herald-Token header"));
        }

        if let Some(command) = decode_resource(req, path) {
            return command;
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
            ("GET", "/api/screens") => Ok(Command::GetScreens),
            ("GET", "/api/log") => Ok(Command::GetLog),
            ("POST", "/api/restart") => Ok(Command::Restart),
            ("POST", OTA_PATH) => Err(reject(
                501,
                "firmware updates are only possible on the device",
            )),
            ("POST", "/api/view") => {
                let req: ViewRequest = parse_json(req.body)?;
                Ok(Command::ShowView(req.view))
            }
            ("POST", "/api/display") => {
                let req: DisplayRequest = parse_json(req.body)?;
                Ok(Command::Display { on: req.on })
            }
            ("PUT" | "POST", "/api/weather") => {
                Ok(Command::SetWeather(Some(parse_json(req.body)?)))
            }
            ("DELETE", "/api/weather") => Ok(Command::SetWeather(None)),
            ("POST", "/api/tap") => {
                let req: TapRequest = parse_json(req.body)?;
                if !screen().contains(Point::new(req.x, req.y)) {
                    return Err(reject(400, "x/y must lie on the screen"));
                }
                let p = Point::new(req.x, req.y);
                Ok(Command::Touch(if req.long {
                    Gesture::LongPress(p)
                } else {
                    Gesture::Tap(p)
                }))
            }
            ("POST", "/api/swipe") => {
                let req: SwipeRequest = parse_json(req.body)?;
                let dir = match req.dir.as_str() {
                    "left" => Dir::Left,
                    "right" => Dir::Right,
                    "up" => Dir::Up,
                    "down" => Dir::Down,
                    _ => return Err(reject(400, "dir must be left, right, up or down")),
                };
                Ok(Command::Touch(Gesture::Swipe(dir)))
            }
            ("POST", "/api/wifi") => {
                let req: WifiRequest = parse_json(req.body)?;
                Ok(Command::SetWifi {
                    ssid: req.ssid,
                    password: req.password,
                })
            }
            (
                _,
                "/api/settings" | "/api/items" | "/api/view" | "/api/wifi" | "/api/state"
                | "/api/tap" | "/api/swipe" | "/api/weather" | "/api/screens" | "/api/log"
                | "/api/restart" | "/api/display" | OTA_PATH,
            ) => Err(reject(405, "method not allowed")),
            _ => Err(reject(404, "not found")),
        }
    }
}

/// Routes with a name in the path: `/api/items/<id>`, `/api/screens/<name>`.
fn decode_resource(req: &Request<'_>, path: &str) -> Option<Result<Command, Rejection>> {
    if let Some(id) = path.strip_prefix("/api/items/") {
        return Some(match req.method {
            "PUT" | "POST" => check_item_id(id).and_then(|()| {
                Ok(Command::PutItem {
                    id: id.into(),
                    item: parse_item(req.body)?,
                })
            }),
            "DELETE" => Ok(Command::DeleteItem(id.into())),
            _ => Err(reject(405, "method not allowed")),
        });
    }
    let name = path.strip_prefix("/api/screens/")?;
    Some(match req.method {
        "PUT" | "POST" => parse_json(req.body).map(|screen| Command::PutScreen {
            name: name.into(),
            screen,
            publish: true,
        }),
        "DELETE" => Ok(Command::DeleteScreen {
            name: name.into(),
            publish: true,
        }),
        _ => Err(reject(405, "method not allowed")),
    })
}

/// `ETag` of the state: its content without what changes every second
/// (uptime, item ages to the second), so unchanged polls cost a 304.
fn state_etag(state: &Value) -> String {
    let mut stable = state.clone();
    if let Some(fields) = stable.as_object_mut() {
        fields.remove("uptime_s");
        if let Some(Value::Array(items)) = fields.get_mut("items") {
            for item in items.iter_mut().filter_map(Value::as_object_mut) {
                for key in ["age_s", "remaining_s"] {
                    if let Some(v) = item.get_mut(key) {
                        *v = json!(v.as_u64().map(|s| s / 60));
                    }
                }
            }
        }
    }
    format!("\"{:016x}\"", gfx::str_key(&stable.to_string()))
}

/// Handles one HTTP request.
pub fn handle(app: &mut App, env: &Env, req: &Request<'_>) -> Response {
    let path = req.path.split('?').next().unwrap_or("");
    if req.method == "GET" {
        if path == "/mqtt" {
            return Response::redirect("/#connections");
        }
        // Captive portal: in setup mode every page leads to the app, so
        // phones open it on their own (probe URLs like /generate_204).
        let setup = matches!(env.net, NetState::Setup { .. });
        if setup && path != "/" && path != "/index.html" && !path.starts_with("/api/") {
            return Response::redirect("/");
        }
    }
    match command::handle(&Http, app, env, req) {
        Ok(Reply::Asset {
            body,
            content_type,
            etag,
        }) => {
            if req.if_none_match == Some(etag) {
                return Response::not_modified(etag.to_owned());
            }
            Response {
                status: 200,
                content_type,
                headers: vec![
                    ("Content-Encoding", "gzip".to_owned()),
                    ("ETag", etag.to_owned()),
                    ("Cache-Control", "no-cache".to_owned()),
                ],
                body: Cow::Borrowed(body),
            }
        }
        Ok(Reply::Json(value)) if path == "/api/state" => {
            let etag = state_etag(&value);
            if req.if_none_match == Some(etag.as_str()) {
                return Response::not_modified(etag);
            }
            let mut r = Response::json(200, &value);
            r.headers.push(("ETag", etag));
            r.headers.push(("Cache-Control", "no-cache".to_owned()));
            r
        }
        Ok(Reply::Json(value)) => Response::json(200, &value),
        Ok(Reply::Text(text)) => Response {
            status: 200,
            content_type: "text/plain; charset=utf-8",
            headers: vec![("Cache-Control", "no-store".to_owned())],
            body: Cow::Owned(text.into_bytes()),
        },
        Err(e) => Response::json(e.status, &json!({ "error": e.message })),
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
                if_none_match: None,
            },
        );
        let value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
        (r.status, value)
    }

    fn get(a: &mut App, env: &Env, path: &str, if_none_match: Option<&str>) -> Response {
        handle(
            a,
            env,
            &Request {
                method: "GET",
                path,
                body: b"",
                token: None,
                if_none_match,
            },
        )
    }

    #[test]
    fn serves_the_page_gzipped_with_an_etag() {
        use std::io::Read;
        let mut a = app();
        let r = get(&mut a, &env(0), "/", None);
        assert_eq!(r.status, 200);
        assert!(r.content_type.starts_with("text/html"));
        assert_eq!(r.header("Content-Encoding"), Some("gzip"));
        let mut html = String::new();
        flate2::read::GzDecoder::new(&r.body[..])
            .read_to_string(&mut html)
            .unwrap();
        assert!(html.contains("<title>"), "the body is the gzipped page");

        let etag = r.header("ETag").unwrap().to_owned();
        let again = get(&mut a, &env(0), "/", Some(&etag));
        assert_eq!(again.status, 304, "unchanged page is not sent again");
        assert!(again.body.is_empty());
    }

    #[test]
    fn unchanged_state_answers_304() {
        let mut a = app();
        let r = get(&mut a, &env(1_000), "/api/state", None);
        let etag = r.header("ETag").unwrap().to_owned();
        assert_eq!(
            get(&mut a, &env(5_000), "/api/state", Some(&etag)).status,
            304,
            "uptime alone is no change"
        );
        call(&mut a, 5_000, "PUT", "/api/items/x", r#"{"text":"x"}"#);
        assert_eq!(
            get(&mut a, &env(5_000), "/api/state", Some(&etag)).status,
            200
        );
    }

    #[test]
    fn setup_mode_leads_every_page_to_the_app() {
        let mut a = app();
        let setup = Env {
            net: NetState::Setup {
                ssid: "Herald-1".into(),
                password: "x".into(),
                ip: "192.168.71.1".into(),
            },
            ..env(0)
        };
        for probe in ["/generate_204", "/hotspot-detect.html", "/connecttest.txt"] {
            let r = get(&mut a, &setup, probe, None);
            assert_eq!(
                (r.status, r.header("Location")),
                (302, Some("/")),
                "{probe}"
            );
        }
        assert_eq!(get(&mut a, &env(0), "/generate_204", None).status, 404);
        assert_eq!(
            get(&mut a, &env(0), "/mqtt", None).header("Location"),
            Some("/#connections")
        );
    }

    #[test]
    fn screens_over_http_are_published_retained() {
        let mut a = app();
        let body = r#"{"title":"Energy","widgets":[{"type":"value","value":3.2,"unit":"kW"}]}"#;
        assert_eq!(call(&mut a, 0, "PUT", "/api/screens/energy", body).0, 200);
        assert!(matches!(
            a.take_effects().as_slice(),
            [Effect::PublishScreen { name, payload: Some(_) }] if name == "energy"
        ));
        let (_, screens) = call(&mut a, 0, "GET", "/api/screens", "");
        assert_eq!(screens["energy"]["title"], "Energy");
        let (_, state) = call(&mut a, 0, "GET", "/api/state", "");
        assert_eq!(state["views"][3]["name"], "energy");
        assert_eq!(
            call(&mut a, 0, "POST", "/api/view", r#"{"view":"energy"}"#).0,
            200
        );
        assert_eq!(call(&mut a, 0, "GET", "/api/state", "").1["view"], "energy");

        assert_eq!(call(&mut a, 0, "PUT", "/api/screens/Bad Name", body).0, 400);
        assert_eq!(call(&mut a, 0, "DELETE", "/api/screens/energy", "").0, 200);
        assert!(matches!(
            a.take_effects().as_slice(),
            [Effect::PublishScreen { payload: None, .. }]
        ));
        assert_eq!(
            call(&mut a, 0, "GET", "/api/state", "").1["view"],
            "home",
            "back home"
        );
    }

    #[test]
    fn display_log_and_restart() {
        let mut a = app();
        call(&mut a, 0, "POST", "/api/display", r#"{"on":false}"#);
        assert!(a.is_asleep());
        call(&mut a, 0, "POST", "/api/display", r#"{"on":true}"#);
        assert!(!a.is_asleep());
        let r = get(&mut a, &env(0), "/api/log", None);
        assert!(r.content_type.starts_with("text/plain"));
        call(&mut a, 0, "POST", "/api/restart", "");
        assert_eq!(a.take_effects(), [Effect::Restart]);
        assert_eq!(call(&mut a, 0, "POST", "/api/ota", "").0, 501);
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
            "[1,2]",
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
        assert_eq!(
            call(&mut a, 0, "GET", "/mqtt", "").0,
            302,
            "old docs URL leads to the app"
        );
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
        assert_eq!(a.view(), crate::model::ViewId::Forecast);
        for gone in ["timer", "pomodoro", "clock", "tv"] {
            let body = format!(r#"{{"view":"{gone}"}}"#);
            assert_eq!(
                call(&mut a, 0, "POST", "/api/view", &body).0,
                404,
                "{gone}: no such view"
            );
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
            ("/api/items/ok", "{}"),
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
        assert_eq!(a.view(), crate::model::ViewId::Home);
        assert!(
            a.take_effects()
                .contains(&Effect::Beep(crate::model::Beep::Notice))
        );

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
        let body = r#"{"temp":9.8,"condition":"cloudy","is_day":false,"min":-5.6,"max":10.25,"sunrise":"07:41","sunset":"16:12"}"#;
        assert_eq!(call(&mut a, 0, "PUT", "/api/weather", body).0, 200);
        let (_, state) = call(&mut a, 0, "GET", "/api/state", "");
        let w = &state["weather"];
        assert_eq!(
            w.to_string(),
            r#"{"condition":"cloudy","days":[],"hours":[],"is_day":false,"max_c":10.3,"min_c":-5.6,"sunrise":"07:41","sunset":"16:12","temp_c":9.8,"updated":null}"#
        );
        assert_eq!(call(&mut a, 0, "PUT", "/api/weather", "{}").0, 400);

        // Weather that is not refreshed disappears instead of going stale.
        a.tick(&env(crate::weather::STALE_MS), None);
        assert!(a.weather().is_none());
        assert_eq!(call(&mut a, 0, "DELETE", "/api/weather", "").0, 200);
    }

    #[test]
    fn rotation_settings_round_trip() {
        let mut a = app();
        let body = r#"{"rotate":true,"rotate_secs":30,"rotate_views":["home","forecast"],"rotate_from":"08:00","rotate_until":"18:00"}"#;
        let (status, got) = call(&mut a, 0, "PUT", "/api/settings", body);
        assert_eq!(status, 200, "{got}");
        assert_eq!(got["rotate_views"], json!(["home", "forecast"]));
        let bad = r#"{"rotate_views":["home","Not a view!"]}"#;
        assert_eq!(call(&mut a, 0, "PUT", "/api/settings", bad).0, 400);
    }

    #[test]
    fn web_preview_taps_and_swipes() {
        let mut a = app();
        assert_eq!(a.view(), crate::model::ViewId::Home);
        let (status, _) = call(&mut a, 1_000, "POST", "/api/swipe", r#"{"dir":"left"}"#);
        assert_eq!(status, 200);
        assert_eq!(a.view(), crate::model::ViewId::Forecast);
        call(&mut a, 1_100, "POST", "/api/swipe", r#"{"dir":"right"}"#);
        assert_eq!(a.view(), crate::model::ViewId::Home);

        // A long press opens the menu, as on the screen.
        call(
            &mut a,
            1_200,
            "POST",
            "/api/tap",
            r#"{"x":240,"y":160,"long":true}"#,
        );
        assert!(a.menu_open, "long press opens the menu");

        for (path, body) in [
            ("/api/tap", r#"{"x":480,"y":0}"#),
            ("/api/tap", r#"{"x":-1,"y":0}"#),
            ("/api/swipe", r#"{"dir":"sideways"}"#),
        ] {
            assert_eq!(
                call(&mut a, 1_300, "POST", path, body).0,
                400,
                "{path} {body}"
            );
        }
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
