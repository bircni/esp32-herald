//! MQTT, modelled on AWTRIX: everything lives under one topic prefix, feed
//! items are topics of their own, and an empty payload removes one. The
//! platforms own the MQTT client; this module maps topics to commands and
//! decides what to publish.
//!
//! | Topic                  | Payload                              |
//! |------------------------|--------------------------------------|
//! | `<prefix>/item/<id>`   | item JSON as for HTTP; empty removes |
//! | `<prefix>/clear`       | anything: empties the feed           |
//! | `<prefix>/view`        | `home` or `{"view":"home"}`          |
//! | `<prefix>/settings`    | any subset of the settings           |
//!
//! The device publishes `<prefix>/state` (retained JSON, as `GET
//! /api/state`) and `<prefix>/status` (`online`, or `offline` as last will).

use serde::Deserialize;
use serde_json::Value;

use crate::{
    app::App,
    command::{check_item_id, parse_json, reject, state, Command, Rejection, Transport},
    gfx,
    model::{Env, Settings, ViewId},
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
    pub fn from_settings(s: &Settings) -> Option<Self> {
        (!s.mqtt_host.is_empty()).then(|| Self {
            host: s.mqtt_host.clone(),
            port: s.mqtt_port,
            user: s.mqtt_user.clone(),
            password: s.mqtt_password.clone(),
            prefix: s.mqtt_prefix.clone(),
        })
    }

    pub fn url(&self) -> String {
        format!("mqtt://{}:{}", self.host, self.port)
    }

    pub fn subscriptions(&self) -> [String; 4] {
        let p = &self.prefix;
        [
            format!("{p}/item/+"),
            format!("{p}/clear"),
            format!("{p}/view"),
            format!("{p}/settings"),
        ]
    }

    /// `online` / `offline`, retained; `offline` is the last will.
    pub fn status_topic(&self) -> String {
        format!("{}/status", self.prefix)
    }

    pub fn state_topic(&self) -> String {
        format!("{}/state", self.prefix)
    }

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
            return Ok(if msg.payload.iter().all(u8::is_ascii_whitespace) {
                Command::DeleteItem(id.into())
            } else {
                Command::PutItem {
                    id: id.into(),
                    item: parse_json(msg.payload)?,
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
                ViewId::from_name(&name)
                    .map(Command::ShowView)
                    .ok_or_else(|| reject(400, format!("unknown view: {name}")))
            }
            "settings" => Ok(Command::PatchSettings(parse_json(msg.payload)?)),
            _ => Err(reject(404, format!("unknown topic: {}", msg.topic))),
        }
    }
}

/// Decides when `<prefix>/state` is published.
#[derive(Default)]
pub struct StatePublisher {
    last_key: Option<i64>,
    last_ms: u64,
}

impl StatePublisher {
    /// Publish again on the next call, e.g. after reconnecting.
    pub fn reset(&mut self) {
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
        command::{handle, Reply},
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
            prefix: "home/buddy".into(),
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
        send(&mut a, 0, "home/buddy/item/window.kitchen", body).unwrap();
        assert_eq!(a.feed()[0].id, "window.kitchen");
        assert_eq!(a.feed()[0].color, ItemColor::Red);

        send(&mut a, 0, "home/buddy/item/window.kitchen", "").unwrap();
        assert!(a.feed().is_empty(), "an empty payload removes the item");

        send(&mut a, 0, "home/buddy/item/a", r#"{"text":"a"}"#).unwrap();
        send(&mut a, 0, "home/buddy/item/b", r#"{"text":"b"}"#).unwrap();
        send(&mut a, 0, "home/buddy/clear", "").unwrap();
        assert!(a.feed().is_empty());
    }

    #[test]
    fn view_accepts_plain_text_and_json() {
        let mut a = app();
        send(&mut a, 0, "home/buddy/view", "forecast").unwrap();
        assert_eq!(a.view(), ViewId::Forecast);
        send(&mut a, 0, "home/buddy/view", r#"{"view":"calendar"}"#).unwrap();
        assert_eq!(a.view(), ViewId::Calendar);
        assert_eq!(
            send(&mut a, 0, "home/buddy/view", "tv").unwrap_err().status,
            400
        );
    }

    #[test]
    fn settings_by_topic_ignore_the_http_token() {
        let mut a = app();
        send(&mut a, 0, "home/buddy/settings", r#"{"token":"s3cret"}"#).unwrap();
        send(&mut a, 0, "home/buddy/settings", r#"{"rotate":true}"#).unwrap();
        assert!(a.settings().rotate);
    }

    #[test]
    fn bad_messages_are_rejected() {
        let mut a = app();
        for (topic, payload, status) in [
            ("other/item/x", r#"{"text":"x"}"#, 404),
            ("home/buddyx/item/x", r#"{"text":"x"}"#, 404),
            ("home/buddy/item/a b", r#"{"text":"x"}"#, 400),
            ("home/buddy/item/x", "not json", 400),
            ("home/buddy/nope", "", 404),
            ("home/buddy/settings", r#"{"nope":1}"#, 400),
            ("home/buddy/state", "{}", 404),
        ] {
            let got = send(&mut a, 0, topic, payload).unwrap_err();
            assert_eq!(got.status, status, "{topic} {payload}: {got:?}");
        }
        let huge = format!(r#"{{"text":"{}"}}"#, "x".repeat(MAX_PAYLOAD));
        assert_eq!(
            send(&mut a, 0, "home/buddy/item/x", &huge)
                .unwrap_err()
                .status,
            413
        );
        assert!(a.feed().is_empty());
    }

    #[test]
    fn config_follows_the_settings() {
        let mut s = Settings::default();
        assert_eq!(Config::from_settings(&s), None, "off without a host");
        s.mqtt_host = "broker.lan".into();
        let c = Config::from_settings(&s).unwrap();
        assert_eq!(c.url(), "mqtt://broker.lan:1883");
        assert_eq!(c.subscriptions()[0], "deskbuddy/item/+");
        assert_eq!(c.status_topic(), "deskbuddy/status");
        assert_eq!(c.state_topic(), "deskbuddy/state");
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

        send(&mut a, 6_000, "home/buddy/item/x", r#"{"text":"x"}"#).unwrap();
        let payload = p.due(&a, &env(6_000)).expect("new item");
        assert!(payload.contains(r#""id":"x""#));
        assert!(p.due(&a, &env(30_000)).is_none(), "ageing is no news");
        assert!(p.due(&a, &env(66_000)).is_some(), "but a minute passed");

        p.reset();
        assert!(p.due(&a, &env(66_100)).is_some(), "after reconnecting");
    }
}
