//! The UI repaints only the regions it believes have changed. This test
//! drives the app through every screen and checks, tick by tick, that the
//! incrementally updated display is identical to a full repaint. A region
//! that is too small or keyed on too little shows up here as stale pixels.

use std::convert::Infallible;

use embedded_graphics::{pixelcolor::Rgb565, prelude::*};
use ui::{api, App, Env, LocalTime, NetState, RawTouch, Settings, ViewId, SCREEN_H, SCREEN_W};

#[derive(Clone, PartialEq)]
struct Frame(Vec<Rgb565>);

impl Frame {
    fn new() -> Self {
        // Not a colour the UI uses, so unpainted pixels are noticed.
        Frame(vec![Rgb565::new(31, 0, 31); (SCREEN_W * SCREEN_H) as usize])
    }

    fn first_difference(&self, other: &Frame) -> Option<(i32, i32)> {
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
        Rig {
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

    fn advance_wall_clock(&mut self) {
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
            },
        );
        assert_eq!(r.status, 200, "{method} {path} {body}: {}", r.body);
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

const FORECAST: &str = r#"{"current":{"temperature_2m":6.4,"weather_code":3,"is_day":0},"hourly":{"time":["2026-11-21T17:00","2026-11-21T18:00","2026-11-21T19:00","2026-11-21T20:00","2026-11-21T21:00","2026-11-21T22:00","2026-11-21T23:00","2026-11-22T00:00","2026-11-22T01:00","2026-11-22T02:00","2026-11-22T03:00","2026-11-22T04:00"],"temperature_2m":[6.4,6.2,5.9,5.7,5.5,5.3,5.2,4.9,4.8,4.8,4.7,4.5],"precipitation_probability":[23,40,35,18,13,10,20,33,45,45,28,13],"weather_code":[3,3,80,80,61,3,3,3,61,61,3,2],"is_day":[0,0,0,0,0,0,0,0,0,0,0,0]},"daily":{"time":["2026-11-21","2026-11-22","2026-11-23","2026-11-24"],"weather_code":[80,61,77,3],"temperature_2m_max":[6.7,4.9,3.0,1.7],"temperature_2m_min":[0.6,2.1,-0.6,-1.0],"sunrise":["2026-11-21T07:38","2026-11-22T07:39","2026-11-23T07:41","2026-11-24T07:43"],"sunset":["2026-11-21T16:05","2026-11-22T16:04","2026-11-23T16:03","2026-11-24T16:02"]}}"#;

#[test]
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
    r.post(
        "/api/settings",
        r#"{"location":"Berlin","latitude":52.52,"longitude":13.41}"#,
    );
    r.run("weather loading", 300, None);
    r.app.set_weather(FORECAST, &r.env).unwrap();
    r.check("weather");
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
    r.post(
        "/api/settings",
        r#"{"location":"","latitude":0,"longitude":0}"#,
    );
    r.run("calendar without daylight", 300, None);
    r.post("/api/view", r#"{"view":"forecast"}"#);
    r.run("forecast without location", 300, None);

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
        ssid: "Buddy-A1B2".into(),
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
