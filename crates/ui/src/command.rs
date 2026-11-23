//! What other machines can ask the device to do, independent of how the
//! request arrived.
//!
//! Each way in (HTTP, MQTT) implements [`Transport`]: it
//! turns its own messages into a [`Command`], which [`execute`] carries out
//! the same way for all of them.

use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};

use crate::{
    app::App,
    color::Color,
    de::opt_number,
    feed::{self, Item},
    gesture::Gesture,
    logbuf,
    model::{Effect, Env, NetState, Settings},
    screen::Screen,
    views::icons::Symbol,
    weather,
};

const ITEM_TTL_MAX: u32 = 7 * 86_400;
const ITEM_AGE_MAX: u64 = 365 * 86_400;
/// `since` values before 2001 are mistakes, e.g. a duration.
const SINCE_MIN: i64 = 1_000_000_000;

/// The web app, gzipped at build time (see `build.rs`).
pub mod web {
    include!(concat!(env!("OUT_DIR"), "/web.rs"));
}

#[derive(Debug, PartialEq)]
pub enum Command {
    /// The web app.
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
    /// A built-in view or custom screen by name.
    ShowView(String),
    /// Replaces the weather; `None` removes it.
    SetWeather(Option<weather::Pushed>),
    /// A tap, long press or swipe made on the web page's mirror.
    Touch(Gesture),
    /// Wakes the display, or sends it to sleep.
    Display {
        on: bool,
    },
    GetScreens,
    /// Adds or replaces a custom screen. `publish` asks for it to be
    /// published retained on MQTT, for screens that came in over HTTP.
    PutScreen {
        name: String,
        screen: Screen,
        publish: bool,
    },
    DeleteScreen {
        name: String,
        publish: bool,
    },
    /// The recent log lines.
    GetLog,
    Restart,
    SetWifi {
        ssid: String,
        password: String,
    },
}

/// A feed item as sent by other machines. Plain text instead of JSON is
/// taken as `text`, see [`parse_item`].
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemBody {
    pub text: String,
    /// A name, `#RRGGBB` or `[r, g, b]`; mapped to the nearest feed colour.
    #[serde(default)]
    pub color: Option<Color>,
    #[serde(default)]
    pub icon: Option<Symbol>,
    #[serde(default)]
    pub detail: String,
    /// Seconds the reported state has already lasted.
    #[serde(default, deserialize_with = "opt_number")]
    pub age: Option<u64>,
    /// Unix time (seconds) the reported state began. Better than `age` for
    /// retained MQTT messages, which may be delivered much later.
    #[serde(default, deserialize_with = "opt_number")]
    pub since: Option<i64>,
    /// Seconds until the item disappears on its own.
    #[serde(default, deserialize_with = "opt_number")]
    pub ttl: Option<u32>,
    /// Wake the display and show the home view.
    #[serde(default)]
    pub wake: bool,
    #[serde(default)]
    pub beep: bool,
}

impl ItemBody {
    fn text(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            color: None,
            icon: None,
            detail: String::new(),
            age: None,
            since: None,
            ttl: None,
            wake: false,
            beep: false,
        }
    }
}

/// An item from a message body: JSON, or plain text as the item's text.
///
/// # Errors
/// Invalid JSON, or text that is not UTF-8.
pub fn parse_item(body: &[u8]) -> Result<ItemBody, Rejection> {
    let text = std::str::from_utf8(body).map_err(|_| reject(400, "item must be UTF-8 text"))?;
    if text.trim_start().starts_with('{') {
        parse_json(body)
    } else {
        Ok(ItemBody::text(text.trim()))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    Json(Value),
    /// A static file, already gzipped.
    Asset {
        body: &'static [u8],
        content_type: &'static str,
        etag: &'static str,
    },
    Text(String),
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
    ///
    /// # Errors
    /// When the message is malformed or not allowed.
    fn decode(&self, app: &App, message: &Self::Message<'_>) -> Result<Command, Rejection>;
}

/// Decodes a message with `transport` and carries it out.
///
/// # Errors
/// When the message is rejected while decoding or carrying it out.
pub fn handle<T: Transport>(
    transport: &T,
    app: &mut App,
    env: &Env,
    message: &T::Message<'_>,
) -> Result<Reply, Rejection> {
    let command = transport.decode(app, message)?;
    execute(app, env, command)
}

/// # Errors
/// Invalid JSON, or JSON of the wrong shape.
pub fn parse_json<T: DeserializeOwned>(body: &[u8]) -> Result<T, Rejection> {
    serde_json::from_slice(body).map_err(|e| reject(400, format!("invalid JSON body: {e}")))
}

const fn ok() -> Value {
    Value::Bool(true)
}

/// Carries out a command.
///
/// # Errors
/// When the command's content is invalid or cannot be carried out.
pub fn execute(app: &mut App, env: &Env, command: Command) -> Result<Reply, Rejection> {
    let done = match command {
        Command::Page => {
            return Ok(Reply::Asset {
                body: web::INDEX_GZ,
                content_type: "text/html; charset=utf-8",
                etag: web::INDEX_ETAG,
            });
        }
        Command::GetLog => return Ok(Reply::Text(logbuf::text())),
        Command::State => return Ok(Reply::Json(state(app, env))),
        Command::GetSettings => return Ok(Reply::Json(json!(app.settings()))),
        Command::PatchSettings(patch) => return patch_settings(app, patch).map(Reply::Json),
        Command::GetItems => return Ok(Reply::Json(items_json(app, env))),
        Command::GetScreens => return Ok(Reply::Json(screens_json(app))),
        Command::PutItem { id, item } => put_item(app, env, id, &item)?,
        Command::DeleteItem(id) => {
            if !app.remove_item(&id) {
                return Err(reject(404, format!("no item {id:?}")));
            }
            ok()
        }
        Command::ClearItems => {
            app.clear_items();
            ok()
        }
        Command::ShowView(name) => {
            let view = app
                .view_by_name(&name)
                .ok_or_else(|| reject(404, format!("no view or screen {name:?}")))?;
            app.set_view(view, env.now_ms);
            app.wake(env.now_ms);
            return Ok(Reply::Json(json!({ "view": app.view_name(view) })));
        }
        Command::SetWeather(Some(pushed)) => {
            let w = pushed.into_weather().map_err(|e| reject(400, e))?;
            app.set_weather(w, env);
            ok()
        }
        Command::SetWeather(None) => {
            app.clear_weather();
            ok()
        }
        Command::Touch(gesture) => {
            if let Gesture::Tap(p) | Gesture::LongPress(p) = gesture
                && !app.profile().area().contains(p)
            {
                return Err(reject(400, "x/y must lie on the screen"));
            }
            if let Some(action) = crate::touch::gesture_action(gesture) {
                app.act(action, env);
            }
            ok()
        }
        Command::Display { on } => {
            if on {
                app.wake(env.now_ms);
            } else {
                app.sleep();
            }
            ok()
        }
        Command::PutScreen {
            name,
            screen,
            publish,
        } => {
            let payload = publish.then(|| serde_json::to_string(&screen).unwrap_or_default());
            app.put_screen(&name, screen, env.now_ms)
                .map_err(|e| reject(400, e))?;
            if let Some(payload) = payload {
                app.push_effect(Effect::PublishScreen {
                    name,
                    payload: Some(payload),
                });
            }
            ok()
        }
        // Idempotent: clearing a screen on the broker echoes back here.
        Command::DeleteScreen { name, publish } => {
            app.remove_screen(&name);
            if publish {
                app.push_effect(Effect::PublishScreen {
                    name,
                    payload: None,
                });
            }
            ok()
        }
        Command::Restart => {
            app.push_effect(Effect::Restart);
            ok()
        }
        Command::SetWifi { ssid, password } => set_wifi(app, ssid, password)?,
    };
    Ok(Reply::Json(json!({ "ok": done })))
}

fn patch_settings(app: &mut App, patch: Map<String, Value>) -> Result<Value, Rejection> {
    let mut merged = json!(app.settings());
    let Some(fields) = merged.as_object_mut() else {
        return Err(reject(500, "settings are not an object"));
    };
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

/// # Errors
/// When `id` is not a valid item id.
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

fn put_item(app: &mut App, env: &Env, id: String, req: &ItemBody) -> Result<Value, Rejection> {
    check_item_id(&id)?;
    let text = feed::clean(&req.text, feed::TEXT_MAX);
    if text.trim().is_empty() {
        return Err(reject(400, "text must not be empty"));
    }
    if let Some(ttl) = req.ttl
        && !(1..=ITEM_TTL_MAX).contains(&ttl)
    {
        return Err(reject(
            400,
            format!("ttl must be 1..={ITEM_TTL_MAX} seconds"),
        ));
    }
    let age_s = match (req.age, req.since) {
        (Some(_), Some(_)) => return Err(reject(400, "send either age or since, not both")),
        (Some(age), None) => Some(age),
        (None, Some(since)) if since < SINCE_MIN => {
            return Err(reject(
                400,
                "since must be a Unix time in seconds, e.g. 1795000000; \
                 for how long something has lasted, send age",
            ));
        }
        // Without a synchronised clock the start time cannot be placed;
        // the item then counts from now.
        (None, Some(since)) => env
            .unix_s
            .map(|now| now.saturating_sub(since).max(0) as u64),
        (None, None) => None,
    };
    let backdate_ms = age_s.unwrap_or(0).min(ITEM_AGE_MAX) * 1000;
    app.put_item(
        Item {
            id,
            text,
            detail: feed::clean(&req.detail, feed::DETAIL_MAX),
            color: req.color.map_or_else(Default::default, Color::item),
            icon: req.icon,
            since_ms: env.now_ms as i64 - backdate_ms as i64,
            until_ms: req.ttl.map(|ttl| env.now_ms + u64::from(ttl) * 1000),
        },
        age_s.is_some(),
    )
    .map_err(|e| reject(409, e))?;
    if req.wake || req.beep {
        app.alert(req.beep, env.now_ms);
    }
    Ok(ok())
}

fn set_wifi(app: &mut App, ssid: String, password: String) -> Result<Value, Rejection> {
    if ssid.is_empty() || ssid.len() > 32 {
        return Err(reject(400, "ssid must be 1..=32 bytes"));
    }
    if !(password.is_empty() || (8..=63).contains(&password.len())) {
        return Err(reject(400, "password must be empty or 8..=63 characters"));
    }
    app.push_effect(Effect::SetWifi { ssid, password });
    Ok(ok())
}

#[must_use]
pub fn items_json(app: &App, env: &Env) -> Value {
    app.feed()
        .iter()
        .map(|i| {
            json!({
                "id": i.id,
                "text": i.text,
                "detail": i.detail,
                "color": i.color,
                "icon": i.icon,
                "age_s": i.age_ms(env.now_ms) / 1000,
                "remaining_s": i.until_ms.map(|u| u.saturating_sub(env.now_ms) / 1000),
            })
        })
        .collect()
}

/// All custom screens by name, as they would be sent.
#[must_use]
pub fn screens_json(app: &App) -> Value {
    let screens = app.screens();
    screens
        .ordered()
        .into_iter()
        .filter_map(|slot| screens.get(slot))
        .map(|e| (e.name.clone(), json!(e.screen)))
        .collect::<Map<String, Value>>()
        .into()
}

/// One decimal place, without the noise of an `f32` widened to `f64`
/// (9.8 would otherwise be printed as 9.800000190734863).
fn round1(v: f32) -> f64 {
    (f64::from(v) * 10.0).round() / 10.0
}

fn hhmm((h, m): (u8, u8)) -> String {
    format!("{h:02}:{m:02}")
}

/// Snapshot of the device. Contains no secrets.
#[must_use]
pub fn state(app: &App, env: &Env) -> Value {
    let net = match &env.net {
        NetState::Offline => json!({ "state": "offline" }),
        NetState::Connecting { ssid } => json!({ "state": "connecting", "ssid": ssid }),
        NetState::Connected { ip } => json!({ "state": "connected", "ip": ip, "rssi": env.rssi }),
        NetState::Setup { ssid, .. } => json!({ "state": "setup", "ssid": ssid }),
    };
    let mqtt = match env.mqtt {
        None => "off",
        Some(false) => "disconnected",
        Some(true) => "connected",
    };
    let s = app.settings();
    let views: Vec<Value> = app
        .nav_order()
        .into_iter()
        .map(|v| json!({ "name": app.view_name(v), "custom": v.built_in_name().is_none() }))
        .collect();
    json!({
        "version": crate::VERSION,
        "uptime_s": env.now_ms / 1000,
        "view": app.view_name(app.view()),
        "views": views,
        "asleep": app.is_asleep(),
        "night": app.is_night(),
        "token_required": !s.token.is_empty(),
        "net": net,
        "mqtt": mqtt,
        "mqtt_prefix": s.mqtt_prefix,
        "hostname": s.hostname,
        "time": env.local.map(|t| hhmm((t.hour, t.minute))),
        "date": env.local.map(|t| json!({
            "year": t.year, "month": t.month, "day": t.day, "weekday": t.weekday,
        })),
        "rotate": s.rotate,
        "theme": s.theme,
        "accent": s.accent_color,
        "brightness": s.brightness,
        "heap_free": env.heap_free,
        "firmware": { "slot": env.firmware_slot, "ota": env.firmware_slot.is_some() },
        "screens_rev": app.screens().rev().to_string(),
        // What the board can do; the web app shows only what applies.
        "device": app.profile(),
        // Boards that keep their picture: the web app shows /api/frame.bmp
        // (changes with this) instead of drawing the screen itself.
        "frame": app.shown_frame.as_ref().map(|(rev, _)| rev.to_string()),
        "items": items_json(app, env),
        "weather": app.weather().map(|w| json!({
            "temp_c": round1(w.temp_c),
            "min_c": round1(w.min_c),
            "max_c": round1(w.max_c),
            "condition": w.condition,
            "is_day": w.is_day,
            "updated": w.updated.map(hhmm),
            "sunrise": w.sunrise.map(hhmm),
            "sunset": w.sunset.map(hhmm),
            "hours": w.upcoming(env.local.as_ref()).iter().map(|h| json!({
                "hour": h.hour, "temp_c": round1(h.temp_c), "rain_pct": h.rain_pct,
                "condition": h.condition, "is_day": h.is_day,
            })).collect::<Vec<_>>(),
            "days": w.days.iter().map(|d| json!({
                "weekday": d.weekday, "condition": d.condition,
                "min_c": round1(d.min_c), "max_c": round1(d.max_c),
            })).collect::<Vec<_>>(),
        })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Calibration;

    fn app() -> App {
        App::new(
            crate::Profile::FREENOVE_35,
            Settings::default(),
            Some(Calibration::identity(480, 320)),
        )
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
    fn state_reports_the_board() {
        let env = Env::default();
        let s = state(&app(), &env);
        assert_eq!(s["device"]["name"], "freenove-35");
        assert_eq!(s["device"]["touch"], true);
        assert!(s["frame"].is_null(), "the TFT keeps no frame");

        let mut paper = App::new(crate::Profile::CROWPANEL_579, Settings::default(), None);
        let s = state(&paper, &env);
        assert_eq!(
            (
                s["device"]["width"].as_i64(),
                s["device"]["backlight"].as_bool()
            ),
            (Some(272), Some(false))
        );
        let frame = crate::epaper::Renderer::new(50).render(&paper, &env);
        paper.set_shown_frame(frame);
        assert_eq!(state(&paper, &env)["frame"], "1");
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

    #[test]
    fn numbers_may_come_as_strings() {
        let mut a = app();
        let env = Env {
            now_ms: 10_000,
            unix_s: Some(1_800_000_000),
            ..Env::default()
        };
        // As sent by a Home Assistant template.
        put(
            &mut a,
            &env,
            "s",
            r#"{"text":"x","since":"1799996400","ttl":" 60 "}"#,
        )
        .unwrap();
        let s = a.feed().iter().find(|i| i.id == "s").unwrap();
        assert_eq!(s.age_ms(10_000), 3_600_000);
        assert_eq!(s.until_ms, Some(70_000));

        // A duration sent as since is a mistake, not 1970.
        for since in ["0", "5"] {
            let body = format!(r#"{{"text":"zu","color":"green","since":"{since}"}}"#);
            let err = put(&mut a, &env, "z", &body).unwrap_err();
            assert!(err.message.contains("age"), "{}", err.message);
        }

        put(&mut a, &env, "e", r#"{"text":"x","age":""}"#).unwrap();
        for bad in [
            r#"{"text":"x","since":"soon"}"#,
            r#"{"text":"x","ttl":true}"#,
        ] {
            assert!(serde_json::from_str::<ItemBody>(bad).is_err(), "{bad}");
        }
    }
}
