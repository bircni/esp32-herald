//! MQTT, modelled on AWTRIX: one topic prefix, feed items as topics.
//!
//! What goes to the device and what comes from it never share a topic, so a
//! sender does not hear its own commands back as readings.
//!
//! The platforms own the MQTT client; this module maps topics to commands and
//! decides what to publish.
//!
//! To the device, shared by every display on the prefix:
//!
//! | Topic                       | Payload                              |
//! |-----------------------------|--------------------------------------|
//! | `<prefix>/data/item/<id>`   | item JSON as for HTTP; empty removes |
//! | `<prefix>/data/screen/<name>` | custom screen JSON; empty removes  |
//! | `<prefix>/data/weather`     | weather JSON, see [`crate::weather::Pushed`]; empty removes |
//! | `<prefix>/cmd/clear`        | anything: empties the feed           |
//! | `<prefix>/cmd/screen`       | `home` or `{"screen":"home"}`: what to show |
//! | `<prefix>/cmd/settings`     | any subset of the settings           |
//!
//! Whatever arrives is shown; the device does not care whether the broker
//! kept a message (retained) or it was just sent.
//!
//! From the device, under `<prefix>/state/<hostname>/`: `status` (`online`,
//! or `offline` as last will; the only retained one), `state`, `settings`
//! and `screen` (each published when it changes), `error` (why a message
//! was rejected, see [`error_payload`]) and `log` (warnings and errors).

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    app::App,
    command::{Command, Rejection, Transport, check_item_id, parse_item, parse_json, reject},
    model::{Env, NetState, Settings},
};

/// Payloads larger than this are rejected.
pub const MAX_PAYLOAD: usize = 4096;
pub const ONLINE: &str = "online";
pub const OFFLINE: &str = "offline";

/// Connection settings; `None` from [`Config::from_settings`] means MQTT is off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub prefix: String,
    /// Names this device's own topics under `<prefix>/state/`.
    pub hostname: String,
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
            hostname: s.hostname.clone(),
        })
    }

    #[must_use]
    pub fn url(&self) -> String {
        format!("mqtt://{}:{}", self.host, self.port)
    }

    #[must_use]
    pub fn subscriptions(&self) -> [String; 4] {
        let p = &self.prefix;
        [
            format!("{p}/data/item/+"),
            format!("{p}/data/screen/+"),
            format!("{p}/data/weather"),
            format!("{p}/cmd/+"),
        ]
    }

    /// One of this device's own topics.
    fn own(&self, name: &str) -> String {
        format!("{}/state/{}/{name}", self.prefix, self.hostname)
    }

    /// Where a custom screen lives, retained.
    #[must_use]
    pub fn screen_topic(&self, name: &str) -> String {
        format!("{}/data/screen/{name}", self.prefix)
    }

    /// `online` / `offline`, retained; `offline` is the last will.
    #[must_use]
    pub fn status_topic(&self) -> String {
        self.own("status")
    }

    /// Rejected messages are reported here, not retained.
    #[must_use]
    pub fn error_topic(&self) -> String {
        self.own("error")
    }

    /// Warnings and errors from the device's log, not retained.
    #[must_use]
    pub fn log_topic(&self) -> String {
        self.own("log")
    }

    #[must_use]
    pub fn report_topic(&self, report: Report) -> String {
        self.own(report.name())
    }

    /// Retained topics of older versions, cleared on connecting so they
    /// do not linger on the broker.
    #[must_use]
    pub fn old_topics(&self) -> [String; 2] {
        let p = &self.prefix;
        [format!("{p}/state"), format!("{p}/status")]
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
struct ScreenPayload {
    screen: String,
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

        if let Some(id) = topic.strip_prefix("data/item/") {
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
        if let Some(name) = topic.strip_prefix("data/screen/") {
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
            "data/weather" if is_blank(msg.payload) => Ok(Command::SetWeather(None)),
            "data/weather" => Ok(Command::SetWeather(Some(parse_json(msg.payload)?))),
            "cmd/clear" => Ok(Command::ClearItems),
            "cmd/screen" => {
                let text = std::str::from_utf8(msg.payload)
                    .map_err(|_| reject(400, "screen must be text"))?
                    .trim();
                let name = if text.starts_with('{') {
                    parse_json::<ScreenPayload>(msg.payload)?.screen
                } else {
                    text.to_owned()
                };
                Ok(Command::ShowView(name))
            }
            "cmd/settings" => Ok(Command::PatchSettings(parse_json(msg.payload)?)),
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

/// What the device reports about itself, each on its own topic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    /// A short summary: version, view, network, how many items.
    State,
    /// The settings, without secrets.
    Settings,
    /// The name of the view or custom screen on the display.
    Screen,
}

impl Report {
    pub const ALL: [Self; 3] = [Self::State, Self::Settings, Self::Screen];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::State => "state",
            Self::Settings => "settings",
            Self::Screen => "screen",
        }
    }

    /// The payload. Leaves out what changes all the time (uptime, clock,
    /// signal strength), so a new payload means news.
    #[must_use]
    pub fn payload(self, app: &App, env: &Env) -> String {
        match self {
            Self::State => {
                let net = match &env.net {
                    NetState::Offline => json!({ "state": "offline" }),
                    NetState::Connecting { ssid } => json!({ "state": "connecting", "ssid": ssid }),
                    NetState::Connected { ip } => json!({ "state": "connected", "ip": ip }),
                    NetState::Setup { ssid, .. } => json!({ "state": "setup", "ssid": ssid }),
                };
                json!({
                    "version": crate::VERSION,
                    "device": app.screen().name,
                    "view": app.view_name(app.view()),
                    "asleep": app.is_asleep(),
                    "night": app.is_night(),
                    "net": net,
                    "items": app.feed().len(),
                    "weather": app.weather().is_some(),
                    "firmware_slot": env.firmware_slot,
                })
                .to_string()
            }
            Self::Settings => {
                let mut settings = json!(app.settings());
                if let Value::Object(fields) = &mut settings {
                    fields.remove("token");
                    fields.remove("mqtt_password");
                }
                settings.to_string()
            }
            Self::Screen => app.view_name(app.view()),
        }
    }
}

/// Decides when the reports go out: each when its payload changes, and all
/// of them after (re)connecting. Nothing is published on a timer.
#[derive(Default)]
pub struct Reporter {
    last: [Option<String>; Report::ALL.len()],
}

impl Reporter {
    /// Publish everything on the next call, e.g. after reconnecting.
    pub fn reset(&mut self) {
        self.last = Default::default();
    }

    /// The reports whose payload changed since they were last returned.
    pub fn due(&mut self, app: &App, env: &Env) -> Vec<(Report, String)> {
        Report::ALL
            .iter()
            .zip(&mut self.last)
            .filter_map(|(&report, last)| {
                let payload = report.payload(app, env);
                (last.as_deref() != Some(payload.as_str())).then(|| {
                    *last = Some(payload.clone());
                    (report, payload)
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        command::{Reply, handle},
        model::{Calibration, Env, ItemColor},
    };
    use serde_json::Value;

    fn app() -> App {
        App::new(
            crate::Profile::FREENOVE_35,
            Settings::default(),
            Some(Calibration::identity(480, 320)),
        )
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
        send(&mut a, 0, "home/herald/data/item/window.kitchen", body).unwrap();
        assert_eq!(a.feed()[0].id, "window.kitchen");
        assert_eq!(a.feed()[0].color, ItemColor::Red);

        send(&mut a, 0, "home/herald/data/item/window.kitchen", "").unwrap();
        assert!(a.feed().is_empty(), "an empty payload removes the item");

        send(&mut a, 0, "home/herald/data/item/a", r#"{"text":"a"}"#).unwrap();
        send(&mut a, 0, "home/herald/data/item/b", r#"{"text":"b"}"#).unwrap();
        send(&mut a, 0, "home/herald/cmd/clear", "").unwrap();
        assert!(a.feed().is_empty());
    }

    #[test]
    fn weather_comes_by_topic() {
        let mut a = app();
        let body = r#"{"temp":4.2,"condition":"rainy"}"#;
        send(&mut a, 0, "home/herald/data/weather", body).unwrap();
        let w = a.weather().unwrap();
        assert_eq!(w.temp_c, 4.2);

        let bad = send(
            &mut a,
            0,
            "home/herald/data/weather",
            r#"{"temp":400,"condition":"sunny"}"#,
        );
        assert_eq!(bad.unwrap_err().status, 400);
        assert!(a.weather().is_some(), "a bad message keeps the weather");

        send(&mut a, 0, "home/herald/data/weather", "").unwrap();
        assert!(a.weather().is_none());
    }

    #[test]
    fn items_may_be_plain_text_with_any_colour() {
        let mut a = app();
        send(&mut a, 0, "home/herald/data/item/door", "Front door open").unwrap();
        assert_eq!(a.feed()[0].text, "Front door open");
        let body = r##"{"text":"Hot","color":"#FF2000","icon":"temperature"}"##;
        send(&mut a, 0, "home/herald/data/item/heat", body).unwrap();
        let heat = a.feed().iter().find(|i| i.id == "heat").unwrap();
        assert_eq!(
            heat.color,
            crate::model::ItemColor::Red,
            "nearest feed colour"
        );
        assert_eq!(heat.icon, Some(crate::views::icons::Symbol::Temperature));
    }

    #[test]
    fn screens_come_and_go_by_topic() {
        let mut a = app();
        let body = r#"{"widgets":[{"type":"gauge","label":"CO2","value":"812","max":2000}]}"#;
        send(&mut a, 0, "home/herald/data/screen/air", body).unwrap();
        assert!(
            a.take_effects().is_empty(),
            "already on the broker, not published again"
        );
        send(&mut a, 0, "home/herald/cmd/screen", "air").unwrap();
        assert_eq!(a.view_name(a.view()), "air");
        send(&mut a, 0, "home/herald/data/screen/air", "").unwrap();
        send(&mut a, 0, "home/herald/data/screen/air", "").unwrap();
        assert_eq!(
            a.view(),
            crate::model::ViewId::Home,
            "removing it goes home"
        );
    }

    #[test]
    fn screen_accepts_plain_text_and_json() {
        let mut a = app();
        send(&mut a, 0, "home/herald/cmd/screen", "forecast").unwrap();
        assert_eq!(a.view(), crate::model::ViewId::Forecast);
        send(
            &mut a,
            0,
            "home/herald/cmd/screen",
            r#"{"screen":"calendar"}"#,
        )
        .unwrap();
        assert_eq!(a.view(), crate::model::ViewId::Calendar);
        assert_eq!(
            send(&mut a, 0, "home/herald/cmd/screen", "tv")
                .unwrap_err()
                .status,
            404
        );
    }

    #[test]
    fn settings_by_topic_ignore_the_http_token() {
        let mut a = app();
        send(
            &mut a,
            0,
            "home/herald/cmd/settings",
            r#"{"token":"s3cret"}"#,
        )
        .unwrap();
        send(&mut a, 0, "home/herald/cmd/settings", r#"{"rotate":true}"#).unwrap();
        assert!(a.settings().rotate);
    }

    #[test]
    fn bad_messages_are_rejected() {
        let mut a = app();
        for (topic, payload, status) in [
            ("other/item/x", r#"{"text":"x"}"#, 404),
            ("home/heraldx/item/x", r#"{"text":"x"}"#, 404),
            ("home/herald/data/item/a b", r#"{"text":"x"}"#, 400),
            ("home/herald/data/item/x", r#"{"text": broken"#, 400),
            ("home/herald/data/screen/x", r#"{"widgets":[]}"#, 400),
            ("home/herald/nope", "", 404),
            ("home/herald/cmd/nope", "", 404),
            ("home/herald/cmd/settings", r#"{"nope":1}"#, 400),
            // Topics of older versions, and the device's own.
            ("home/herald/item/x", r#"{"text":"x"}"#, 404),
            ("home/herald/view", "home", 404),
            ("home/herald/state/herald/screen", "home", 404),
        ] {
            let got = send(&mut a, 0, topic, payload).unwrap_err();
            assert_eq!(got.status, status, "{topic} {payload}: {got:?}");
        }
        let huge = format!(r#"{{"text":"{}"}}"#, "x".repeat(MAX_PAYLOAD));
        assert_eq!(
            send(&mut a, 0, "home/herald/data/item/x", &huge)
                .unwrap_err()
                .status,
            413
        );
        assert!(a.feed().is_empty());
    }

    #[test]
    fn rejections_are_reported_with_the_payload() {
        let mut a = app();
        let topic = "home/herald/data/item/window";
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
        s.hostname = "hall".into();
        let c = Config::from_settings(&s).unwrap();
        assert_eq!(c.url(), "mqtt://broker.lan:1883");
        assert_eq!(
            c.subscriptions(),
            [
                "herald/data/item/+",
                "herald/data/screen/+",
                "herald/data/weather",
                "herald/cmd/+",
            ]
        );
        assert_eq!(c.screen_topic("air"), "herald/data/screen/air");
        assert_eq!(c.status_topic(), "herald/state/hall/status");
        assert_eq!(c.error_topic(), "herald/state/hall/error");
        assert_eq!(c.log_topic(), "herald/state/hall/log");
        assert_eq!(c.report_topic(Report::Screen), "herald/state/hall/screen");
        assert_eq!(c.old_topics(), ["herald/state", "herald/status"]);
    }

    #[test]
    fn reports_go_out_when_they_change() {
        let mut a = app();
        let env = Env::default();
        let mut reporter = Reporter::default();
        let first: Vec<Report> = reporter.due(&a, &env).into_iter().map(|r| r.0).collect();
        assert_eq!(first, Report::ALL, "everything once");
        assert!(reporter.due(&a, &env).is_empty(), "nothing without news");

        let later = Env {
            now_ms: 3_600_000,
            rssi: Some(-80),
            ..Env::default()
        };
        assert!(
            reporter.due(&a, &later).is_empty(),
            "uptime and signal are no news"
        );

        send(&mut a, 0, "home/herald/cmd/screen", "forecast").unwrap();
        let due = reporter.due(&a, &env);
        assert_eq!(
            due.iter().map(|r| r.0).collect::<Vec<_>>(),
            [Report::State, Report::Screen]
        );
        assert_eq!(due[1].1, "forecast");

        reporter.reset();
        assert_eq!(reporter.due(&a, &env).len(), 3, "again after reconnecting");
    }

    #[test]
    fn settings_reports_hold_no_secrets() {
        let s = Settings {
            token: "s3cret".into(),
            mqtt_password: "hunter2".into(),
            ..Settings::default()
        };
        let a = App::new(crate::Profile::FREENOVE_35, s, None);
        let payload = Report::Settings.payload(&a, &Env::default());
        assert!(!payload.contains("s3cret") && !payload.contains("hunter2"));
        assert!(payload.contains("mqtt_prefix"));
    }
}
