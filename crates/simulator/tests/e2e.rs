//! End-to-end: starts the simulator headless and uses it over HTTP the way
//! the web app, Home Assistant and scripts do.
#![expect(
    clippy::expect_used,
    reason = "test helpers: a failed request should stop the test with a message"
)]

use std::{
    net::TcpListener,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use ureq::{Agent, Body, http::Response};

/// The simulator process; killed when the test ends, also on failure.
struct Sim {
    child: Child,
    base: String,
    agent: Agent,
}

impl Drop for Sim {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Sim {
    fn start() -> Self {
        let port = TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .expect("a free port")
            .port();
        let child = Command::new(env!("CARGO_BIN_EXE_simulator"))
            .args(["--headless", "--port", &port.to_string()])
            .stdout(Stdio::null())
            .spawn()
            .expect("simulator starts");
        let agent: Agent = Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .into();
        let sim = Self {
            child,
            base: format!("http://127.0.0.1:{port}"),
            agent,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while sim.agent.get(sim.url("/api/state")).call().is_err() {
            assert!(Instant::now() < deadline, "simulator did not come up");
            thread::sleep(Duration::from_millis(50));
        }
        sim
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn get(&self, path: &str) -> Response<Body> {
        self.agent.get(self.url(path)).call().expect("GET answered")
    }

    fn send(&self, method: &str, path: &str, body: &str) -> (u16, Value) {
        let url = self.url(path);
        let mut r = match method {
            "PUT" => self.agent.put(url).send(body),
            "POST" => self.agent.post(url).send(body),
            "DELETE" => self.agent.delete(url).call(),
            other => panic!("method {other}"),
        }
        .expect("request answered");
        let text = r.body_mut().read_to_string().unwrap_or_default();
        (
            r.status().as_u16(),
            serde_json::from_str(&text).unwrap_or(Value::Null),
        )
    }

    fn state(&self) -> Value {
        let mut r = self.get("/api/state");
        serde_json::from_str(&r.body_mut().read_to_string().expect("state body"))
            .expect("state JSON")
    }
}

fn header(r: &Response<Body>, name: &str) -> Option<String> {
    r.headers().get(name)?.to_str().ok().map(str::to_owned)
}

#[test]
fn page_and_state_are_cached_by_etag() {
    let sim = Sim::start();
    let mut page = sim.get("/");
    assert_eq!(page.status(), 200);
    let etag = header(&page, "etag").expect("page has an ETag");
    let html = page.body_mut().read_to_string().expect("page body");
    assert!(
        html.contains("<title>Herald"),
        "the gzipped page arrives whole"
    );

    let again = sim
        .agent
        .get(sim.url("/"))
        .header("If-None-Match", &etag)
        .call()
        .expect("answered");
    assert_eq!(again.status(), 304, "a reload costs nothing");

    let state = sim.get("/api/state");
    let etag = header(&state, "etag").expect("state has an ETag");
    let unchanged = sim
        .agent
        .get(sim.url("/api/state"))
        .header("If-None-Match", &etag)
        .call()
        .expect("answered");
    assert_eq!(unchanged.status(), 304);

    let old = sim.get("/mqtt");
    assert_eq!(
        (old.status().as_u16(), header(&old, "location").as_deref()),
        (302, Some("/#connections"))
    );
}

#[test]
fn items_weather_and_screens_reach_the_device() {
    let sim = Sim::start();
    assert_eq!(sim.send("PUT", "/api/items/door", "Front door open").0, 200);
    let body = r##"{"text":"Too hot","color":"#FF2000","icon":"temperature","ttl":"600"}"##;
    assert_eq!(sim.send("PUT", "/api/items/heat", body).0, 200);
    let items = sim.state()["items"].clone();
    let by_id = |id: &str| {
        items
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == id)
            .cloned()
            .unwrap()
    };
    assert_eq!(by_id("door")["text"], "Front door open");
    assert_eq!(by_id("heat")["color"], "red");
    assert_eq!(by_id("heat")["icon"], "temperature");

    let weather = r#"{"temp":"4.2","condition":"rainy","min":1,"max":6,"location":"ignored"}"#;
    assert_eq!(sim.send("PUT", "/api/weather", weather).0, 200);
    assert_eq!(sim.state()["weather"]["condition"], "rain");

    let screen = json!({"title": "Energy", "widgets": [
        {"type": "value", "label": "Solar", "value": 3.2, "unit": "kW"},
        {"type": "gauge", "label": "Battery", "value": 64, "unit": "%"},
        {"type": "chart", "values": [0, 1.5, 3.2], "style": "bar"},
        {"type": "countdown", "label": "Train", "until": 4_000_000_000_i64},
    ]});
    let (status, answer) = sim.send("PUT", "/api/screens/energy", &screen.to_string());
    assert_eq!(status, 200, "{answer}");
    let state = sim.state();
    assert!(
        state["views"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["name"] == "energy" && v["custom"] == true)
    );
    let (status, shown) = sim.send("POST", "/api/view", r#"{"view":"energy"}"#);
    assert_eq!((status, shown["view"].as_str()), (200, Some("energy")));
    // The device keeps drawing it; the screen survives a few ticks.
    thread::sleep(Duration::from_millis(200));
    assert_eq!(sim.state()["view"], "energy");

    let (status, err) = sim.send("PUT", "/api/screens/broken", r#"{"widgets":[]}"#);
    assert_eq!(status, 400);
    assert!(err["error"].as_str().unwrap().contains("widgets"), "{err}");

    assert_eq!(sim.send("DELETE", "/api/screens/energy", "").0, 200);
    assert_eq!(sim.state()["view"], "home", "a removed screen goes home");
}

#[test]
fn settings_gestures_and_log() {
    let sim = Sim::start();
    let night =
        r#"{"night":true,"night_from":"22:00","night_until":"06:30","night_brightness":10}"#;
    let (status, saved) = sim.send("PUT", "/api/settings", night);
    assert_eq!(
        (status, saved["night_until"].as_str()),
        (200, Some("06:30"))
    );
    let (status, err) = sim.send("PUT", "/api/settings", r#"{"night_brightness":200}"#);
    assert_eq!(status, 400);
    assert!(err["error"].as_str().unwrap().contains("night_brightness"));

    assert_eq!(sim.send("POST", "/api/swipe", r#"{"dir":"left"}"#).0, 200);
    assert_eq!(sim.state()["view"], "forecast");
    assert_eq!(sim.send("POST", "/api/display", r#"{"on":false}"#).0, 200);
    assert_eq!(sim.state()["asleep"], true);
    assert_eq!(sim.send("POST", "/api/tap", r#"{"x":240,"y":160}"#).0, 200);
    assert_eq!(sim.state()["asleep"], false, "a tap on the mirror wakes it");

    assert_eq!(
        sim.send("POST", "/api/ota", "").0,
        501,
        "no OTA in the simulator"
    );
    let mut log = sim.get("/api/log");
    assert!(
        header(&log, "content-type")
            .unwrap()
            .starts_with("text/plain")
    );
    let _ = log.body_mut().read_to_string();
}
