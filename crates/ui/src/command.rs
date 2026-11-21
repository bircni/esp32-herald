//! What other machines can ask the device to do, independent of how the
//! request arrived. Each way in (HTTP, MQTT) implements [`Transport`]: it
//! turns its own messages into a [`Command`], which [`execute`] carries out
//! the same way for all of them.

use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Map, Value};

use crate::{
    app::App,
    feed::{self, Item},
    model::{Effect, Env, ItemColor, NetState, Settings, ViewId},
};

const ITEM_TTL_MAX: u32 = 7 * 86_400;
const ITEM_AGE_MAX: u64 = 365 * 86_400;
const INDEX_HTML: &str = include_str!("../web/index.html");

#[derive(Debug, PartialEq)]
pub enum Command {
    /// The settings web page.
    Page,
    State,
    GetSettings,
    /// Changes the named settings, keeps the others.
    PatchSettings(Map<String, Value>),
    GetItems,
    PutItem {
        id: String,
        item: ItemBody,
    },
    DeleteItem(String),
    ClearItems,
    ShowView(ViewId),
    SetWifi {
        ssid: String,
        password: String,
    },
}

/// A feed item as sent by other machines.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemBody {
    pub text: String,
    #[serde(default)]
    pub color: ItemColor,
    #[serde(default)]
    pub detail: String,
    /// Seconds the reported state has already lasted.
    pub age: Option<u64>,
    /// Unix time (seconds) the reported state began. Better than `age` for
    /// retained MQTT messages, which may be delivered much later.
    pub since: Option<i64>,
    /// Seconds until the item disappears on its own.
    pub ttl: Option<u32>,
    /// Wake the display and show the home view.
    #[serde(default)]
    pub wake: bool,
    #[serde(default)]
    pub beep: bool,
}

#[derive(Debug, PartialEq)]
pub enum Reply {
    Json(Value),
    Html(&'static str),
}

/// Why a message was not carried out. `status` follows HTTP conventions.
#[derive(Debug, PartialEq, Eq)]
pub struct Rejection {
    pub status: u16,
    pub message: String,
}

pub fn reject(status: u16, message: impl Into<String>) -> Rejection {
    Rejection {
        status,
        message: message.into(),
    }
}

/// A way for other machines to reach the device.
pub trait Transport {
    /// One incoming message, e.g. an HTTP request or an MQTT publish.
    type Message<'a>;

    /// Turns a message into a command, or rejects it as malformed or not
    /// allowed. Authorisation is part of decoding, as each transport has its
    /// own (a header token for HTTP, the broker's login for MQTT).
    fn decode(&self, app: &App, message: &Self::Message<'_>) -> Result<Command, Rejection>;
}

/// Decodes a message with `transport` and carries it out.
pub fn handle<T: Transport>(
    transport: &T,
    app: &mut App,
    env: &Env,
    message: &T::Message<'_>,
) -> Result<Reply, Rejection> {
    let command = transport.decode(app, message)?;
    execute(app, env, command)
}

pub fn parse_json<T: DeserializeOwned>(body: &[u8]) -> Result<T, Rejection> {
    serde_json::from_slice(body).map_err(|e| reject(400, format!("invalid JSON body: {e}")))
}

pub fn execute(app: &mut App, env: &Env, command: Command) -> Result<Reply, Rejection> {
    let json = match command {
        Command::Page => return Ok(Reply::Html(INDEX_HTML)),
        Command::State => state(app, env),
        Command::GetSettings => json!(app.settings()),
        Command::PatchSettings(patch) => patch_settings(app, patch)?,
        Command::GetItems => items_json(app, env),
        Command::PutItem { id, item } => put_item(app, env, id, item)?,
        Command::DeleteItem(id) => {
            if !app.remove_item(&id) {
                return Err(reject(404, format!("no item {id:?}")));
            }
            json!({ "ok": true })
        }
        Command::ClearItems => {
            app.clear_items();
            json!({ "ok": true })
        }
        Command::ShowView(view) => {
            app.set_view(view, env.now_ms);
            app.wake(env.now_ms);
            json!({ "view": view.name() })
        }
        Command::SetWifi { ssid, password } => set_wifi(app, ssid, password)?,
    };
    Ok(Reply::Json(json))
}

fn patch_settings(app: &mut App, patch: Map<String, Value>) -> Result<Value, Rejection> {
    let mut merged = json!(app.settings());
    let fields = merged
        .as_object_mut()
        .expect("settings serialize to an object");
    for (key, value) in patch {
        if !fields.contains_key(&key) {
            return Err(reject(400, format!("unknown setting: {key}")));
        }
        fields.insert(key, value);
    }
    let settings: Settings =
        serde_json::from_value(merged).map_err(|e| reject(400, format!("invalid setting: {e}")))?;
    app.update_settings(settings).map_err(|e| reject(400, e))?;
    Ok(json!(app.settings()))
}

pub fn check_item_id(id: &str) -> Result<(), Rejection> {
    if feed::valid_id(id) {
        Ok(())
    } else {
        Err(reject(
            400,
            format!(
                "item id must be 1..={} characters of a-z, A-Z, 0-9, '-', '_', '.'",
                feed::ID_MAX
            ),
        ))
    }
}

fn put_item(app: &mut App, env: &Env, id: String, req: ItemBody) -> Result<Value, Rejection> {
    check_item_id(&id)?;
    let text = feed::clean(&req.text, feed::TEXT_MAX);
    if text.trim().is_empty() {
        return Err(reject(400, "text must not be empty"));
    }
    if let Some(ttl) = req.ttl {
        if !(1..=ITEM_TTL_MAX).contains(&ttl) {
            return Err(reject(
                400,
                format!("ttl must be 1..={ITEM_TTL_MAX} seconds"),
            ));
        }
    }
    let age_s = match (req.age, req.since) {
        (Some(_), Some(_)) => return Err(reject(400, "send either age or since, not both")),
        (Some(age), None) => Some(age),
        // Without a synchronised clock the start time cannot be placed;
        // the item then counts from now.
        (None, Some(since)) => env
            .unix_s
            .map(|now| now.saturating_sub(since).max(0) as u64),
        (None, None) => None,
    };
    let age_ms = age_s.unwrap_or(0).min(ITEM_AGE_MAX) * 1000;
    app.put_item(
        Item {
            id,
            text,
            detail: feed::clean(&req.detail, feed::DETAIL_MAX),
            color: req.color,
            since_ms: env.now_ms as i64 - age_ms as i64,
            until_ms: req.ttl.map(|ttl| env.now_ms + u64::from(ttl) * 1000),
        },
        age_s.is_some(),
    )
    .map_err(|e| reject(409, e))?;
    if req.wake || req.beep {
        app.alert(req.beep, env.now_ms);
    }
    Ok(json!({ "ok": true }))
}

fn set_wifi(app: &mut App, ssid: String, password: String) -> Result<Value, Rejection> {
    if ssid.is_empty() || ssid.len() > 32 {
        return Err(reject(400, "ssid must be 1..=32 bytes"));
    }
    if !(password.is_empty() || (8..=63).contains(&password.len())) {
        return Err(reject(400, "password must be empty or 8..=63 characters"));
    }
    app.push_effect(Effect::SetWifi { ssid, password });
    Ok(json!({ "ok": true, "note": "saved, the device now reconnects" }))
}

pub fn items_json(app: &App, env: &Env) -> Value {
    app.feed()
        .iter()
        .map(|i| {
            json!({
                "id": i.id,
                "text": i.text,
                "detail": i.detail,
                "color": i.color,
                "age_s": i.age_ms(env.now_ms) / 1000,
                "remaining_s": i.until_ms.map(|u| u.saturating_sub(env.now_ms) / 1000),
            })
        })
        .collect()
}

/// One decimal place, without the noise of an `f32` widened to `f64`
/// (9.8 would otherwise be printed as 9.800000190734863).
fn round1(v: f32) -> f64 {
    (f64::from(v) * 10.0).round() / 10.0
}

/// Snapshot of the device. Contains no secrets.
pub fn state(app: &App, env: &Env) -> Value {
    let net = match &env.net {
        NetState::Offline => json!({ "state": "offline" }),
        NetState::Connecting { ssid } => json!({ "state": "connecting", "ssid": ssid }),
        NetState::Connected { ip } => json!({ "state": "connected", "ip": ip }),
        NetState::Setup { ssid, .. } => json!({ "state": "setup", "ssid": ssid }),
    };
    let mqtt = match env.mqtt {
        None => "off",
        Some(false) => "disconnected",
        Some(true) => "connected",
    };
    json!({
        "version": crate::VERSION,
        "uptime_s": env.now_ms / 1000,
        "view": app.view().name(),
        "asleep": app.is_asleep(),
        "token_required": !app.settings().token.is_empty(),
        "net": net,
        "mqtt": mqtt,
        "time": env.local.map(|t| format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second)),
        "rotate": app.settings().rotate,
        "items": items_json(app, env),
        "weather": app.weather().map(|w| json!({
            "temp_c": round1(w.temp_c),
            "min_c": round1(w.min_c),
            "max_c": round1(w.max_c),
            "condition": w.condition,
            "is_day": w.is_day,
            "updated": w.updated.map(|(h, m)| format!("{h:02}:{m:02}")),
            "sunrise": w.sunrise.map(|(h, m)| format!("{h:02}:{m:02}")),
            "sunset": w.sunset.map(|(h, m)| format!("{h:02}:{m:02}")),
        })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Calibration;

    fn app() -> App {
        App::new(Settings::default(), Some(Calibration::identity()))
    }

    fn item(json: &str) -> ItemBody {
        serde_json::from_str(json).unwrap()
    }

    fn put(app: &mut App, env: &Env, id: &str, json: &str) -> Result<Reply, Rejection> {
        execute(
            app,
            env,
            Command::PutItem {
                id: id.into(),
                item: item(json),
            },
        )
    }

    #[test]
    fn since_is_placed_with_the_wall_clock() {
        let mut a = app();
        let env = Env {
            now_ms: 10_000,
            unix_s: Some(1_800_000_000),
            ..Env::default()
        };
        put(&mut a, &env, "w", r#"{"text":"x","since":1799996400}"#).unwrap();
        assert_eq!(a.feed()[0].age_ms(10_000), 3_600_000);

        // In the future (clock skew): counts from now.
        put(&mut a, &env, "f", r#"{"text":"x","since":1800000100}"#).unwrap();
        assert_eq!(
            a.feed()
                .iter()
                .find(|i| i.id == "f")
                .unwrap()
                .age_ms(10_000),
            0
        );

        let unsynced = Env {
            now_ms: 10_000,
            ..Env::default()
        };
        put(&mut a, &unsynced, "u", r#"{"text":"x","since":1799996400}"#).unwrap();
        assert_eq!(
            a.feed()
                .iter()
                .find(|i| i.id == "u")
                .unwrap()
                .age_ms(10_000),
            0
        );

        let both = put(&mut a, &env, "b", r#"{"text":"x","since":1,"age":1}"#);
        assert_eq!(both.unwrap_err().status, 400);
    }
}
