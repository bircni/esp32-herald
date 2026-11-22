//! The UI repaints only the regions it believes have changed. This test
//! drives the app through every screen and checks, tick by tick, that the
//! incrementally updated display is identical to a full repaint. A region
//! that is too small or keyed on too little shows up here as stale pixels.

use std::convert::Infallible;

use embedded_graphics::{pixelcolor::Rgb565, prelude::*};
use ui::{App, Env, LocalTime, NetState, RawTouch, SCREEN_H, SCREEN_W, Settings, ViewId, api};

#[derive(Clone, PartialEq)]
struct Frame(Vec<Rgb565>);

impl Frame {
    fn new() -> Self {
        // Not a colour the UI uses, so unpainted pixels are noticed.
        Self(vec![Rgb565::new(31, 0, 31); (SCREEN_W * SCREEN_H) as usize])
    }

    fn first_difference(&self, other: &Self) -> Option<(i32, i32)> {
        let i = self.0.iter().zip(&other.0).position(|(a, b)| a != b)? as i32;
        Some((i % SCREEN_W, i / SCREEN_W))
    }
}

impl OriginDimensions for Frame {
    fn size(&self) -> Size {
        Size::new(SCREEN_W as u32, SCREEN_H as u32)
    }
}

impl DrawTarget for Frame {
    type Color = Rgb565;
    type Error = Infallible;

    #[expect(
        clippy::panic_in_result_fn,
        reason = "test display: a pixel off the screen fails the test"
    )]
    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(p, color) in pixels {
            assert!(
                (0..SCREEN_W).contains(&p.x) && (0..SCREEN_H).contains(&p.y),
                "pixel outside the screen: {p:?}"
            );
            self.0[(p.y * SCREEN_W + p.x) as usize] = color;
        }
        Ok(())
    }
}

struct Rig {
    app: App,
    env: Env,
    incremental: Frame,
    checks: usize,
}

impl Rig {
    fn new() -> Self {
        Self {
            app: App::new(Settings::default(), None),
            env: Env {
                now_ms: 0,
                local: Some(LocalTime {
                    year: 2026,
                    month: 11,
                    day: 19,
                    weekday: 3,
                    hour: 23,
                    minute: 59,
                    second: 50,
                }),
                net: NetState::Connected {
                    ip: "192.168.1.42".into(),
                },
                ..Env::default()
            },
            incremental: Frame::new(),
            checks: 0,
        }
    }

    /// Draws incrementally, then compares with a full repaint.
    fn check(&mut self, what: &str) {
        if self.app.is_asleep() {
            return; // backlight off, nothing is drawn
        }
        let Ok(()) = self.app.draw(&mut self.incremental, &self.env);
        let mut full = Frame::new();
        self.app.invalidate();
        let Ok(()) = self.app.draw(&mut full, &self.env);
        if let Some((x, y)) = self.incremental.first_difference(&full) {
            panic!(
                "{what}: stale pixel at ({x}, {y}) at t={} ms",
                self.env.now_ms
            );
        }
        self.checks += 1;
    }

    /// Advances the monotonic clock in 30 ms ticks and the wall clock along
    /// with it, checking the display after every tick.
    fn run(&mut self, what: &str, ms: u64, touch: Option<(i32, i32)>) {
        for _ in 0..ms / 30 {
            let before = self.env.now_ms / 1000;
            self.env.now_ms += 30;
            if self.env.now_ms / 1000 != before {
                self.advance_wall_clock();
            }
            let raw = touch.map(|(x, y)| RawTouch {
                x: (x * 11 + 250) as u16,
                y: (y * 7 + 300) as u16,
            });
            self.app.tick(&self.env, raw);
            self.app.take_effects();
            self.check(what);
        }
    }

    const fn advance_wall_clock(&mut self) {
        let Some(t) = self.env.local.as_mut() else {
            return;
        };
        t.second += 1;
        if t.second == 60 {
            t.second = 0;
            t.minute += 1;
        }
        if t.minute == 60 {
            t.minute = 0;
            t.hour += 1;
        }
        if t.hour == 24 {
            t.hour = 0;
            t.day += 1;
            t.weekday = (t.weekday + 1) % 7;
        }
    }

    fn tap(&mut self, what: &str, x: i32, y: i32) {
        self.run(what, 90, Some((x, y)));
        self.run(what, 120, None);
    }

    fn swipe(&mut self, what: &str, from: (i32, i32), to: (i32, i32)) {
        for i in 0..=5 {
            let at = (
                from.0 + (to.0 - from.0) * i / 5,
                from.1 + (to.1 - from.1) * i / 5,
            );
            self.run(what, 30, Some(at));
        }
        self.run(what, 120, None);
    }

    fn request(&mut self, method: &str, path: &str, body: &str) {
        let r = api::handle(
            &mut self.app,
            &self.env,
            &api::Request {
                method,
                path,
                body: body.as_bytes(),
                token: None,
                if_none_match: None,
            },
        );
        assert_eq!(r.status, 200, "{method} {path} {body}: {}", r.text());
        self.app.take_effects();
        self.check(path);
    }

    fn post(&mut self, path: &str, body: &str) {
        self.request("POST", path, body);
    }

    fn put(&mut self, path: &str, body: &str) {
        self.request("PUT", path, body);
    }
}

const FORECAST: &str = r#"{"temp":6.4,"condition":"cloudy","is_day":false,"sunrise":"07:38","sunset":"16:05","hourly":[{"hour":17,"temp":6.4,"rain":23,"condition":"cloudy","is_day":false},{"hour":18,"temp":6.2,"rain":40,"condition":"cloudy","is_day":false},{"hour":19,"temp":5.9,"rain":35,"condition":"rainy","is_day":false},{"hour":20,"temp":5.7,"rain":18,"condition":"rainy","is_day":false},{"hour":21,"temp":5.5,"rain":13,"condition":"rainy","is_day":false},{"hour":22,"temp":5.3,"rain":10,"condition":"cloudy","is_day":false},{"hour":23,"temp":5.2,"rain":20,"condition":"cloudy","is_day":false},{"hour":0,"temp":4.9,"rain":33,"condition":"cloudy","is_day":false},{"hour":1,"temp":4.8,"rain":45,"condition":"rainy","is_day":false},{"hour":2,"temp":4.8,"rain":45,"condition":"rainy","is_day":false},{"hour":3,"temp":4.7,"rain":28,"condition":"cloudy","is_day":false},{"hour":4,"temp":4.5,"rain":13,"condition":"partlycloudy","is_day":false}],"daily":[{"date":"2026-11-21","min":0.6,"max":6.7,"condition":"rainy"},{"date":"2026-11-22","min":2.1,"max":4.9,"condition":"rainy"},{"date":"2026-11-23","min":-0.6,"max":3.0,"condition":"snowy"},{"date":"2026-11-24","min":-1.0,"max":1.7,"condition":"cloudy"}]}"#;

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one scenario from first boot to sleep"
)]
fn incremental_redraw_matches_full_redraw() {
    let mut r = Rig::new();
    let corners = [(30, 30), (450, 30), (450, 290), (30, 290)];

    // First boot: calibration, including one failed attempt.
    r.check("first frame");
    for _ in 0..4 {
        r.run("bad calibration", 300, Some((240, 160)));
        r.run("bad calibration", 120, None);
    }
    for (x, y) in corners {
        r.run("calibration", 300, Some((x, y)));
        r.run("calibration", 120, None);
    }
    assert_eq!(r.app.view(), ViewId::Home);

    // Home: the clock runs across midnight, the feed fills up, ages and
    // overflows, items change colour and disappear.
    r.run("idle home", 12_000, None);
    r.run("no weather yet", 300, None);
    r.put("/api/weather", FORECAST);
    r.run("weather", 300, None);
    r.put("/api/items/claude", r#"{"text":"Claude is working"}"#);
    r.run("one item", 1_000, None);
    r.put(
        "/api/items/window",
        r#"{"text":"Kitchen window open","color":"red","age":3540}"#,
    );
    r.run("item ages past the hour", 61_000, None);
    r.put(
        "/api/items/claude",
        r#"{"text":"Claude needs you","color":"yellow"}"#,
    );
    r.run("item changes", 300, None);
    for i in 0..6 {
        r.put(
            &format!("/api/items/n{i}"),
            &format!(
                r#"{{"text":"A rather long notification number {i}","color":"green","detail":"{i} new","ttl":{}}}"#,
                2 + i
            ),
        );
        r.run("feed grows", 60, None);
    }
    r.run("feed overflows, then shrinks", 9_000, None);
    r.request("DELETE", "/api/items/window", "");
    r.run("item removed", 300, None);
    r.request("DELETE", "/api/items", "");
    r.run("feed empty", 300, None);
    let synced = r.env.local.take();
    r.run("clock unsynced", 300, None);
    r.env.net = NetState::Offline;
    r.run("clock offline", 300, None);
    r.env.net = NetState::Connected {
        ip: "10.0.0.7".into(),
    };
    r.env.local = synced;
    r.run("clock synced again", 300, None);

    // Forecast and calendar, with and without data.
    r.swipe("to forecast", (400, 160), (80, 160));
    assert_eq!(r.app.view(), ViewId::Forecast);
    r.run("forecast", 300, None);
    r.swipe("to calendar", (400, 160), (80, 160));
    assert_eq!(r.app.view(), ViewId::Calendar);
    r.run("calendar", 300, None);
    r.request("DELETE", "/api/weather", "");
    r.run("calendar without daylight", 300, None);
    r.post("/api/view", r#"{"view":"forecast"}"#);
    r.run("forecast without weather", 300, None);

    // Custom screens: every widget, values changing, layouts switching.
    r.put("/api/weather", FORECAST);
    let all = r##"{"title":"Everything","widgets":[
        {"type":"value","label":"Solar","value":3.2,"unit":"kW","color":"yellow"},
        {"type":"gauge","label":"Battery","value":64,"unit":"%","zones":[{"to":20,"color":"red"},{"to":100,"color":"green"}]},
        {"type":"chart","label":"Today","values":[0,0.4,1.8,3.2,2.1],"unit":"kW"},
        {"type":"list","rows":[{"label":"Kitchen","value":21.5,"unit":"°"},{"label":"Bath","value":23}]},
        {"type":"grid","tiles":[{"label":"CO2","value":812},{"label":"Hum","value":"48%","color":"#3366FF"}]},
        {"type":"countdown","label":"Train","until":1795334400}
    ]}"##;
    r.put("/api/screens/all", all);
    r.post("/api/view", r#"{"view":"all"}"#);
    r.run("custom screen", 300, None);
    r.put(
        "/api/screens/all",
        &all.replace(
            "3.2,\"unit\":\"kW\",\"color\"",
            "4.7,\"unit\":\"kW\",\"color\"",
        ),
    );
    r.run("custom screen, new value", 300, None);
    let extras = r#"{"widgets":[
        {"type":"value","value":"1234","digits":true},
        {"type":"gauge","label":"CPU","value":40,"style":"bar"},
        {"type":"progress","label":"Upload","value":70},
        {"type":"text","text":"A longer paragraph that has to wrap over a few lines","size":"small"},
        {"type":"icon","icon":"window","label":"Kitchen","value":"open","color":"red"},
        {"type":"chart","values":[3,1,4,1,5,9,2,6],"style":"bar"}
    ]}"#;
    r.put("/api/screens/extras", extras);
    r.post("/api/view", r#"{"view":"extras"}"#);
    r.run("custom screen with extras", 300, None);
    r.put(
        "/api/screens/clock",
        r#"{"widgets":[{"type":"clock"},{"type":"weather"}]}"#,
    );
    r.post("/api/view", r#"{"view":"clock"}"#);
    r.run("clock and weather widgets", 300, None);
    r.request("DELETE", "/api/screens/clock", "");
    r.run("screen removed", 300, None);
    r.request("DELETE", "/api/screens/all", "");
    r.request("DELETE", "/api/screens/extras", "");

    // Menu: every entry, then back out.
    r.run("long press", 900, Some((240, 160)));
    r.run("release", 120, None);
    r.tap("brightness down", 50, 103);
    r.tap("brightness up", 430, 103);
    r.tap("theme", 120, 163);
    r.tap("rotate", 120, 223);
    r.tap("wifi", 360, 223);
    r.tap("cancel confirm by tapping elsewhere", 240, 30);
    r.tap("theme back", 120, 163);
    r.tap("recalibrate", 360, 163);
    for (x, y) in corners {
        r.run("recalibration", 300, Some((x, y)));
        r.run("recalibration", 120, None);
    }
    r.run("long press", 900, Some((240, 160)));
    r.run("release", 120, None);
    r.tap("rotate off", 120, 223);
    r.tap("close", 240, 283);

    // Theme and colour changes repaint everything.
    r.post(
        "/api/settings",
        r##"{"theme":"light","accent_color":"#E0457B"}"##,
    );
    for view in ["home", "forecast", "calendar"] {
        r.post("/api/view", &format!(r#"{{"view":"{view}"}}"#));
        r.run(view, 300, None);
    }

    // Rotation through every view.
    r.post("/api/settings", r#"{"rotate":true,"rotate_secs":5}"#);
    let before = r.app.view();
    r.run("rotating", 62_000, None);
    assert_ne!(r.app.view(), before, "rotation moved on");

    // WiFi setup mode appears, is dismissed by a tap, and ends.
    r.env.net = NetState::Setup {
        ssid: "Herald-A1B2".into(),
        password: "k7mq2xwp".into(),
        ip: "192.168.71.1".into(),
    };
    r.run("wifi setup", 300, None);
    r.tap("dismiss", 240, 160);
    r.env.net = NetState::Connected {
        ip: "192.168.1.42".into(),
    };
    r.run("connected", 300, None);

    // Sleep, and waking up with a touch that must not act as a swipe.
    r.post("/api/settings", r#"{"sleep_secs":5,"rotate":false}"#);
    r.post("/api/view", r#"{"view":"home"}"#);
    r.run("falling asleep", 6_000, None);
    assert!(r.app.is_asleep());
    r.swipe("wake with a swipe", (400, 160), (80, 160));
    assert!(!r.app.is_asleep());
    assert_eq!(r.app.view(), ViewId::Home, "wake touch was swallowed");

    assert!(r.checks > 2_000, "only {} frames checked", r.checks);
}
