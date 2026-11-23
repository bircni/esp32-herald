//! MQTT, modelled on AWTRIX: everything lives under one topic prefix, feed
//! items are topics of their own, and an empty payload removes one.
//!
//! The platforms own the MQTT client; this module maps topics to commands and
//! decides what to publish.
//!
//! | Topic                  | Payload                              |
//! |------------------------|--------------------------------------|
//! | `<prefix>/item/<id>`   | item JSON as for HTTP; empty removes |
//! | `<prefix>/clear`       | anything: empties the feed           |
//! | `<prefix>/view`        | `home` or `{"view":"home"}`          |
//! | `<prefix>/settings`    | any subset of the settings           |
//! | `<prefix>/weather`     | weather JSON, see [`crate::weather::Pushed`]; empty removes |
//!
//! The device publishes `<prefix>/state` (retained JSON, as `GET
//! /api/state`), `<prefix>/status` (`online`, or `offline` as last will) and
//! `<prefix>/error` (why a message was rejected, see [`error_payload`]).

use serde::Deserialize;
use serde_json::Value;

use crate::{
    app::App,
    command::{
        Command, Rejection, Transport, check_item_id, parse_item, parse_json, reject, state,
    },
    gfx,
    model::{Env, Settings},
};

/// Payloads larger than this are rejected.
pub const MAX_PAYLOAD: usize = 4096;
pub const ONLINE: &str = "online";
pub const OFFLINE: &str = "offline";
/// The state is published when it changes, and at least this often.
const STATE_INTERVAL_MS: u64 = 60_000;

/// Connection settings; `None` from [`Config::from_settings`] means MQTT is off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub prefix: String,
}

impl Config {
    #[must_use]
    pub fn from_settings(s: &Settings) -> Option<Self> {
        (!s.mqtt_host.is_empty()).then(|| Self {
            host: s.mqtt_host.clone(),
            port: s.mqtt_port,
            user: s.mqtt_user.clone(),
            password: s.mqtt_password.clone(),
            prefix: s.mqtt_prefix.clone(),
        })
    }

    #[must_use]
    pub fn url(&self) -> String {
        format!("mqtt://{}:{}", self.host, self.port)
    }

    #[must_use]
    pub fn subscriptions(&self) -> [String; 6] {
        let p = &self.prefix;
        [
            format!("{p}/item/+"),
            format!("{p}/screen/+"),
            format!("{p}/clear"),
            format!("{p}/view"),
            format!("{p}/settings"),
            format!("{p}/weather"),
        ]
    }

    /// Where a custom screen lives, retained.
    #[must_use]
    pub fn screen_topic(&self, name: &str) -> String {
        format!("{}/screen/{name}", self.prefix)
    }

    /// Warnings and errors from the device's log, not retained.
    #[must_use]
    pub fn log_topic(&self) -> String {
        format!("{}/log", self.prefix)
    }

    /// `online` / `offline`, retained; `offline` is the last will.
    #[must_use]
    pub fn status_topic(&self) -> String {
        format!("{}/status", self.prefix)
    }

    #[must_use]
    pub fn state_topic(&self) -> String {
        format!("{}/state", self.prefix)
    }

    /// Rejected messages are reported here, not retained.
    #[must_use]
    pub fn error_topic(&self) -> String {
        format!("{}/error", self.prefix)
    }

    #[must_use]
    pub fn transport(&self) -> Mqtt {
        Mqtt {
            prefix: self.prefix.clone(),
        }
    }
}

/// One received publish.
pub struct Message<'a> {
    pub topic: &'a str,
    pub payload: &'a [u8],
}

/// MQTT as a [`Transport`]. Access control is the broker's job, so the
/// HTTP token does not apply.
pub struct Mqtt {
    pub prefix: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewPayload {
    view: String,
}

impl Transport for Mqtt {
    type Message<'a> = Message<'a>;

    fn decode(&self, _app: &App, msg: &Message<'_>) -> Result<Command, Rejection> {
        if msg.payload.len() > MAX_PAYLOAD {
            return Err(reject(413, "payload too large"));
        }
        let topic = msg
            .topic
            .strip_prefix(self.prefix.as_str())
            .and_then(|t| t.strip_prefix('/'))
            .ok_or_else(|| reject(404, format!("not our topic: {}", msg.topic)))?;

        if let Some(id) = topic.strip_prefix("item/") {
            check_item_id(id)?;
            return Ok(if is_blank(msg.payload) {
                Command::DeleteItem(id.into())
            } else {
                Command::PutItem {
                    id: id.into(),
                    item: parse_item(msg.payload)?,
                }
            });
        }
        if let Some(name) = topic.strip_prefix("screen/") {
            return Ok(if is_blank(msg.payload) {
                Command::DeleteScreen {
                    name: name.into(),
                    publish: false,
                }
            } else {
                Command::PutScreen {
                    name: name.into(),
                    screen: parse_json(msg.payload)?,
                    // Already on the broker.
                    publish: false,
                }
            });
        }
        match topic {
            "clear" => Ok(Command::ClearItems),
            "view" => {
                let text = std::str::from_utf8(msg.payload)
                    .map_err(|_| reject(400, "view must be text"))?
                    .trim();
                let name = if text.starts_with('{') {
                    parse_json::<ViewPayload>(msg.payload)?.view
                } else {
                    text.to_owned()
                };
                Ok(Command::ShowView(name))
            }
            "settings" => Ok(Command::PatchSettings(parse_json(msg.payload)?)),
            "weather" if is_blank(msg.payload) => Ok(Command::SetWeather(None)),
            "weather" => Ok(Command::SetWeather(Some(parse_json(msg.payload)?))),
            _ => Err(reject(404, format!("unknown topic: {}", msg.topic))),
        }
    }
}

fn is_blank(payload: &[u8]) -> bool {
    payload.iter().all(u8::is_ascii_whitespace)
}

/// Longest part of a rejected payload repeated in the error report.
const ERROR_PAYLOAD_MAX: usize = 200;

/// What is published on `<prefix>/error` when a message is rejected: the
/// topic, the reason and the start of the payload, so the sender can fix it.
#[must_use]
pub fn error_payload(msg: &Message<'_>, rejection: &Rejection) -> String {
    let payload = String::from_utf8_lossy(msg.payload);
    let start: String = payload.chars().take(ERROR_PAYLOAD_MAX).collect();
    serde_json::json!({
        "topic": msg.topic,
        "status": rejection.status,
        "error": rejection.message,
        "payload": start,
    })
    .to_string()
}

/// Decides when `<prefix>/state` is published.
#[derive(Default)]
pub struct StatePublisher {
    last_key: Option<i64>,
    last_ms: u64,
}

impl StatePublisher {
    /// Publish again on the next call, e.g. after reconnecting.
    pub const fn reset(&mut self) {
        self.last_key = None;
    }

    /// The payload to publish now, if any.
    pub fn due(&mut self, app: &App, env: &Env) -> Option<String> {
        let mut snapshot = state(app, env);
        let payload = snapshot.to_string();
        // Ignore what changes all the time, so a publish means news.
        if let Some(fields) = snapshot.as_object_mut() {
            fields.remove("uptime_s");
            fields.remove("time");
            if let Some(Value::Array(items)) = fields.get_mut("items") {
                for item in items.iter_mut().filter_map(Value::as_object_mut) {
                    item.remove("age_s");
                    item.remove("remaining_s");
                }
            }
        }
        let key = gfx::str_key(&snapshot.to_string());
        let stale = env.now_ms.saturating_sub(self.last_ms) >= STATE_INTERVAL_MS;
        if self.last_key == Some(key) && !stale {
            return None;
        }
        self.last_key = Some(key);
        self.last_ms = env.now_ms;
        Some(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        command::{Reply, handle},
        model::{Calibration, ItemColor},
    };

    fn app() -> App {
        App::new(Settings::default(), Some(Calibration::identity()))
    }

    fn send(app: &mut App, now_ms: u64, topic: &str, payload: &str) -> Result<Reply, Rejection> {
        let env = Env {
            now_ms,
            ..Env::default()
        };
        let mqtt = Mqtt {
            prefix: "home/herald".into(),
        };
        handle(
            &mqtt,
            app,
            &env,
            &Message {
                topic,
                payload: payload.as_bytes(),
            },
        )
    }

    #[test]
    fn items_are_set_and_removed_by_topic() {
        let mut a = app();
        let body = r#"{"text":"Kitchen window open","color":"red"}"#;
        send(&mut a, 0, "home/herald/item/window.kitchen", body).unwrap();
        assert_eq!(a.feed()[0].id, "window.kitchen");
        assert_eq!(a.feed()[0].color, ItemColor::Red);

        send(&mut a, 0, "home/herald/item/window.kitchen", "").unwrap();
        assert!(a.feed().is_empty(), "an empty payload removes the item");

        send(&mut a, 0, "home/herald/item/a", r#"{"text":"a"}"#).unwrap();
        send(&mut a, 0, "home/herald/item/b", r#"{"text":"b"}"#).unwrap();
        send(&mut a, 0, "home/herald/clear", "").unwrap();
        assert!(a.feed().is_empty());
    }

    #[test]
    fn weather_comes_by_topic() {
        let mut a = app();
        let body = r#"{"temp":4.2,"condition":"rainy"}"#;
        send(&mut a, 0, "home/herald/weather", body).unwrap();
        let w = a.weather().unwrap();
        assert_eq!(w.temp_c, 4.2);

        let bad = send(
            &mut a,
            0,
            "home/herald/weather",
            r#"{"temp":400,"condition":"sunny"}"#,
        );
        assert_eq!(bad.unwrap_err().status, 400);
        assert!(a.weather().is_some(), "a bad message keeps the weather");

        send(&mut a, 0, "home/herald/weather", "").unwrap();
        assert!(a.weather().is_none());
    }

    #[test]
    fn items_may_be_plain_text_with_any_colour() {
        let mut a = app();
        send(&mut a, 0, "home/herald/item/door", "Front door open").unwrap();
        assert_eq!(a.feed()[0].text, "Front door open");
        let body = r##"{"text":"Hot","color":"#FF2000","icon":"temperature"}"##;
        send(&mut a, 0, "home/herald/item/heat", body).unwrap();
        let heat = a.feed().iter().find(|i| i.id == "heat").unwrap();
        assert_eq!(
            heat.color,
            crate::model::ItemColor::Red,
            "nearest feed colour"
        );
        assert_eq!(heat.icon, Some(crate::views::icons::Symbol::Temperature));
    }

    #[test]
    fn screens_come_and_go_by_retained_topic() {
        let mut a = app();
        let body = r#"{"widgets":[{"type":"gauge","label":"CO2","value":"812","max":2000}]}"#;
        send(&mut a, 0, "home/herald/screen/air", body).unwrap();
        assert!(
            a.take_effects().is_empty(),
            "already on the broker, not published again"
        );
        send(&mut a, 0, "home/herald/view", "air").unwrap();
        assert_eq!(a.view_name(a.view()), "air");
        send(&mut a, 0, "home/herald/screen/air", "").unwrap();
        send(&mut a, 0, "home/herald/screen/air", "").unwrap();
        assert_eq!(
            a.view(),
            crate::model::ViewId::Home,
            "removing it goes home"
        );
    }

    #[test]
    fn view_accepts_plain_text_and_json() {
        let mut a = app();
        send(&mut a, 0, "home/herald/view", "forecast").unwrap();
        assert_eq!(a.view(), crate::model::ViewId::Forecast);
        send(&mut a, 0, "home/herald/view", r#"{"view":"calendar"}"#).unwrap();
        assert_eq!(a.view(), crate::model::ViewId::Calendar);
        assert_eq!(
            send(&mut a, 0, "home/herald/view", "tv")
                .unwrap_err()
                .status,
            404
        );
    }

    #[test]
    fn settings_by_topic_ignore_the_http_token() {
        let mut a = app();
        send(&mut a, 0, "home/herald/settings", r#"{"token":"s3cret"}"#).unwrap();
        send(&mut a, 0, "home/herald/settings", r#"{"rotate":true}"#).unwrap();
        assert!(a.settings().rotate);
    }

    #[test]
    fn bad_messages_are_rejected() {
        let mut a = app();
        for (topic, payload, status) in [
            ("other/item/x", r#"{"text":"x"}"#, 404),
            ("home/heraldx/item/x", r#"{"text":"x"}"#, 404),
            ("home/herald/item/a b", r#"{"text":"x"}"#, 400),
            ("home/herald/item/x", r#"{"text": broken"#, 400),
            ("home/herald/screen/x", r#"{"widgets":[]}"#, 400),
            ("home/herald/nope", "", 404),
            ("home/herald/settings", r#"{"nope":1}"#, 400),
            ("home/herald/state", "{}", 404),
        ] {
            let got = send(&mut a, 0, topic, payload).unwrap_err();
            assert_eq!(got.status, status, "{topic} {payload}: {got:?}");
        }
        let huge = format!(r#"{{"text":"{}"}}"#, "x".repeat(MAX_PAYLOAD));
        assert_eq!(
            send(&mut a, 0, "home/herald/item/x", &huge)
                .unwrap_err()
                .status,
            413
        );
        assert!(a.feed().is_empty());
    }

    #[test]
    fn rejections_are_reported_with_the_payload() {
        let mut a = app();
        let topic = "home/herald/item/window";
        let payload = r#"{"text":""Küche", "color":"zu""}"#;
        let err = send(&mut a, 0, topic, payload).unwrap_err();
        let report: Value = serde_json::from_str(&error_payload(
            &Message {
                topic,
                payload: payload.as_bytes(),
            },
            &err,
        ))
        .unwrap();
        assert_eq!(report["topic"], topic);
        assert_eq!(report["status"], 400);
        assert!(report["error"].as_str().unwrap().contains("invalid JSON"));
        assert_eq!(report["payload"], payload);

        let long = "x".repeat(1000);
        let report = error_payload(
            &Message {
                topic,
                payload: long.as_bytes(),
            },
            &err,
        );
        assert!(report.len() < 400, "the payload is cut");
    }

    #[test]
    fn config_follows_the_settings() {
        let mut s = Settings::default();
        assert_eq!(Config::from_settings(&s), None, "off without a host");
        s.mqtt_host = "broker.lan".into();
        let c = Config::from_settings(&s).unwrap();
        assert_eq!(c.url(), "mqtt://broker.lan:1883");
        assert_eq!(c.subscriptions()[0], "herald/item/+");
        assert_eq!(c.status_topic(), "herald/status");
        assert_eq!(c.state_topic(), "herald/state");
        assert_eq!(c.error_topic(), "herald/error");
    }

    #[test]
    fn state_is_published_on_change_and_once_a_minute() {
        let mut a = app();
        let mut p = StatePublisher::default();
        let env = |now_ms| Env {
            now_ms,
            ..Env::default()
        };
        assert!(p.due(&a, &env(0)).is_some(), "first call publishes");
        assert!(p.due(&a, &env(5_000)).is_none(), "uptime alone is no news");

        send(&mut a, 6_000, "home/herald/item/x", r#"{"text":"x"}"#).unwrap();
        let payload = p.due(&a, &env(6_000)).expect("new item");
        assert!(payload.contains(r#""id":"x""#));
        assert!(p.due(&a, &env(30_000)).is_none(), "ageing is no news");
        assert!(p.due(&a, &env(66_000)).is_some(), "but a minute passed");

        p.reset();
        assert!(p.due(&a, &env(66_100)).is_some(), "after reconnecting");
    }
}
