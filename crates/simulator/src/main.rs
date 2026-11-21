//! Desktop simulator: the real UI in an SDL window and the real HTTP API on
//! localhost. Only the hardware drivers are replaced.
//!
//! ```text
//! cargo run -p simulator                      # window + http://127.0.0.1:8080
//! cargo run -p simulator -- --calibrate       # start with the calibration screen
//! cargo run -p simulator -- --setup           # start in WiFi setup mode
//! cargo run -p simulator -- --demo            # sample feed items and a location
//! cargo run -p simulator -- --screenshots DIR # render every screen to PNG, no window
//! ```

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use chrono::{Datelike, Local, Timelike};
use embedded_graphics::{pixelcolor::Rgb565, prelude::*};
use embedded_graphics_simulator::{
    sdl2::MouseButton, OutputSettings, OutputSettingsBuilder, SimulatorDisplay, SimulatorEvent,
    Window,
};
use ui::{
    api, App, Calibration, Effect, Env, Led, LocalTime, NetState, RawTouch, Settings, SCREEN_H,
    SCREEN_W,
};

const SETTINGS_FILE: &str = "sim-settings.json";
const CALIBRATION_FILE: &str = "sim-calibration.json";
const TICK: Duration = Duration::from_millis(30);

struct Shared {
    app: Mutex<App>,
    net: Mutex<NetState>,
    start: Instant,
    /// Where settings are stored; `None` keeps everything in memory.
    storage: Option<PathBuf>,
}

impl Shared {
    fn env(&self) -> Env {
        let now = Local::now();
        Env {
            now_ms: self.start.elapsed().as_millis() as u64,
            local: Some(LocalTime {
                year: now.year() as u16,
                month: now.month() as u8,
                day: now.day() as u8,
                weekday: now.weekday().num_days_from_monday() as u8,
                hour: now.hour() as u8,
                minute: now.minute() as u8,
                second: now.second() as u8,
            }),
            net: self.net.lock().unwrap().clone(),
        }
    }

    /// Carries out what the firmware would do with NVS, the speaker and WiFi.
    fn apply_effects(&self, app: &mut App) {
        for effect in app.take_effects() {
            match effect {
                Effect::SaveSettings => {
                    self.save(SETTINGS_FILE, &serde_json::json!(app.settings()))
                }
                Effect::SaveCalibration(c) => self.save(CALIBRATION_FILE, &serde_json::json!(c)),
                Effect::Beep(beep) => println!("[speaker] {beep:?}"),
                Effect::StartWifiSetup => {
                    println!("[wifi] credentials erased, setup access point opened");
                    *self.net.lock().unwrap() = setup_net();
                }
                Effect::SetWifi { ssid, .. } => {
                    println!("[wifi] credentials for {ssid:?} saved, reconnecting");
                    *self.net.lock().unwrap() = connected_net();
                }
            }
        }
    }

    fn save(&self, file: &str, value: &serde_json::Value) {
        let Some(dir) = &self.storage else { return };
        if let Err(e) = fs::write(dir.join(file), format!("{value:#}\n")) {
            eprintln!("could not write {file}: {e}");
        }
    }
}

fn connected_net() -> NetState {
    NetState::Connected {
        ip: "192.168.1.42".into(),
    }
}

fn setup_net() -> NetState {
    NetState::Setup {
        ssid: "Buddy-A1B2".into(),
        password: "k7mq2xwp".into(),
        ip: "192.168.71.1".into(),
    }
}

fn load<T: serde::de::DeserializeOwned>(dir: &Path, file: &str) -> Option<T> {
    let text = fs::read_to_string(dir.join(file)).ok()?;
    match serde_json::from_str(&text) {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("ignoring {file}: {e}");
            None
        }
    }
}

struct Args {
    calibrate: bool,
    setup: bool,
    demo: bool,
    port: u16,
    screenshots: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        calibrate: false,
        setup: false,
        demo: false,
        port: 8080,
        screenshots: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--calibrate" => args.calibrate = true,
            "--setup" => args.setup = true,
            "--demo" => args.demo = true,
            "--port" => {
                let value = it.next().ok_or("--port needs a number")?;
                args.port = value.parse().map_err(|_| format!("bad port: {value}"))?;
            }
            "--screenshots" => {
                args.screenshots = Some(it.next().ok_or("--screenshots needs a directory")?.into());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(args)
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!(
                "{e}\nusage: simulator [--calibrate] [--setup] [--demo] [--port N] [--screenshots DIR]"
            );
            std::process::exit(2);
        }
    };
    if let Some(dir) = &args.screenshots {
        if let Err(e) = screenshots(dir) {
            eprintln!("screenshots failed: {e}");
            std::process::exit(1);
        }
        return;
    }

    let dir = PathBuf::from(".");
    let settings: Settings = load(&dir, SETTINGS_FILE)
        .filter(|s: &Settings| s.validate().is_ok())
        .unwrap_or_default();
    // The mouse already reports screen coordinates.
    let calibration = (!args.calibrate).then(Calibration::identity);
    let shared = Arc::new(Shared {
        app: Mutex::new(App::new(settings, calibration)),
        net: Mutex::new(if args.setup {
            setup_net()
        } else {
            connected_net()
        }),
        start: Instant::now(),
        storage: Some(dir),
    });

    if args.demo {
        let env = shared.env();
        let mut app = shared.app.lock().unwrap();
        if let Err(e) = demo(&mut app, &env) {
            eprintln!("demo data rejected: {e}");
        }
    }
    serve_http(Arc::clone(&shared), args.port);
    fetch_weather(Arc::clone(&shared));
    run_window(&shared, args.calibrate);
}

/// The same requests a phone, Home Assistant or the Claude Code hook would
/// send, so the views have something to show.
fn demo(app: &mut App, env: &Env) -> Result<(), String> {
    if !app.settings().has_location() {
        post(
            app,
            env,
            "/api/settings",
            r#"{"location":"Berlin","latitude":52.52,"longitude":13.41}"#,
        )?;
    }
    for (id, body) in DEMO_ITEMS {
        request(app, env, "PUT", &format!("/api/items/{id}"), body)?;
    }
    Ok(())
}

const DEMO_ITEMS: [(&str, &str); 4] = [
    (
        "window.kitchen",
        r#"{"text":"Kitchen window open","color":"red","age":1260}"#,
    ),
    (
        "window.bedroom",
        r#"{"text":"Bedroom window tilted","color":"yellow","age":5400}"#,
    ),
    (
        "claude",
        r#"{"text":"Claude is working","color":"blue","age":240}"#,
    ),
    (
        "ci",
        r##"{"text":"CI passed on main","color":"green","detail":"#1284"}"##,
    ),
];

fn request(app: &mut App, env: &Env, method: &str, path: &str, body: &str) -> Result<(), String> {
    let r = api::handle(
        app,
        env,
        &api::Request {
            method,
            path,
            body: body.as_bytes(),
            token: None,
        },
    );
    app.take_effects();
    (r.status == 200)
        .then_some(())
        .ok_or_else(|| format!("{method} {path}: {}", r.body))
}

fn post(app: &mut App, env: &Env, path: &str, body: &str) -> Result<(), String> {
    request(app, env, "POST", path, body)
}

fn output_settings() -> OutputSettings {
    OutputSettingsBuilder::new().build()
}

/// Converts a mouse position into a touch sample. With `fake_adc` the
/// values look like those of a real resistive panel (a different scale and
/// offset per axis), so the calibration screen has real work to do.
fn to_raw(p: Point, fake_adc: bool) -> RawTouch {
    let (x, y) = (p.x.clamp(0, SCREEN_W - 1), p.y.clamp(0, SCREEN_H - 1));
    if fake_adc {
        RawTouch {
            x: (x * 11 + 250) as u16,
            y: (y * 7 + 300) as u16,
        }
    } else {
        RawTouch {
            x: x as u16,
            y: y as u16,
        }
    }
}

fn run_window(shared: &Shared, fake_adc: bool) {
    let size = Size::new(SCREEN_W as u32, SCREEN_H as u32);
    let mut display = SimulatorDisplay::<Rgb565>::new(size);
    let dark = SimulatorDisplay::<Rgb565>::new(size);
    let mut window = Window::new("Deskbuddy simulator", &output_settings());
    let mut touch: Option<Point> = None;
    let mut led = Led::Off;

    loop {
        let frame_start = Instant::now();
        let env = shared.env();
        let asleep = {
            let mut app = shared.app.lock().unwrap();
            let raw = touch.map(|p| to_raw(p, fake_adc));
            app.tick(&env, raw);
            shared.apply_effects(&mut app);
            let Ok(()) = app.draw(&mut display, &env);
            let now_led = app.led();
            if now_led != led {
                led = now_led;
                println!("[led] {led:?}");
            }
            app.backlight() == 0
        };
        // A sleeping display has its backlight off.
        window.update(if asleep { &dark } else { &display });

        for event in window.events() {
            match event {
                SimulatorEvent::Quit => return,
                SimulatorEvent::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    point,
                } => touch = Some(point),
                SimulatorEvent::MouseMove { point } if touch.is_some() => touch = Some(point),
                SimulatorEvent::MouseButtonUp {
                    mouse_btn: MouseButton::Left,
                    ..
                } => touch = None,
                _ => {}
            }
        }
        thread::sleep(TICK.saturating_sub(frame_start.elapsed()));
    }
}

fn serve_http(shared: Arc<Shared>, port: u16) {
    let server = match tiny_http::Server::http(("127.0.0.1", port)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("HTTP API disabled, could not listen on port {port}: {e}");
            return;
        }
    };
    println!("Web page and API: http://127.0.0.1:{port}");
    thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let mut body = Vec::new();
            let limit = api::MAX_BODY as u64 + 1;
            if request
                .as_reader()
                .take(limit)
                .read_to_end(&mut body)
                .is_err()
            {
                continue;
            }
            let token = request
                .headers()
                .iter()
                .find(|h| h.field.equiv(api::TOKEN_HEADER))
                .map(|h| h.value.as_str().to_owned());
            let method = request.method().as_str().to_owned();
            let path = request.url().to_owned();

            let response = {
                let env = shared.env();
                let mut app = shared.app.lock().unwrap();
                let response = api::handle(
                    &mut app,
                    &env,
                    &api::Request {
                        method: &method,
                        path: &path,
                        body: &body,
                        token: token.as_deref(),
                    },
                );
                shared.apply_effects(&mut app);
                response
            };
            let header =
                tiny_http::Header::from_bytes("Content-Type", response.content_type).unwrap();
            let reply = tiny_http::Response::from_string(response.body)
                .with_status_code(response.status)
                .with_header(header);
            if let Err(e) = request.respond(reply) {
                eprintln!("HTTP response failed: {e}");
            }
        }
    });
}

fn fetch_weather(shared: Arc<Shared>) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(1));
        let Some(url) = shared.app.lock().unwrap().weather_due(&shared.env()) else {
            continue;
        };
        // The lock is not held during the request, as on the device.
        let body = ureq::get(&url)
            .call()
            .and_then(|mut r| r.body_mut().read_to_string());
        let result = match body {
            Ok(body) => shared.app.lock().unwrap().set_weather(&body, &shared.env()),
            Err(e) => Err(e.to_string()),
        };
        match result {
            Ok(()) => println!("[weather] updated"),
            Err(e) => eprintln!("[weather] {e}"),
        }
    });
}

// ------------------------------------------------------------ screenshots

/// A real Open-Meteo response (Berlin, 2026-11-21), so screenshots do not
/// depend on the network.
const SAMPLE_WEATHER: &str = r#"{"current":{"temperature_2m":6.4,"weather_code":2,"is_day":1},"hourly":{"time":["2026-11-21T17:00","2026-11-21T18:00","2026-11-21T19:00","2026-11-21T20:00","2026-11-21T21:00","2026-11-21T22:00","2026-11-21T23:00","2026-11-22T00:00","2026-11-22T01:00","2026-11-22T02:00","2026-11-22T03:00","2026-11-22T04:00"],"temperature_2m":[6.4,6.2,5.9,5.7,5.5,5.3,5.2,4.9,4.8,4.8,4.7,4.5],"precipitation_probability":[23,40,35,18,13,10,20,33,45,45,28,13],"weather_code":[3,3,80,80,61,3,3,3,61,61,3,2],"is_day":[0,0,0,0,0,0,0,0,0,0,0,0]},"daily":{"time":["2026-11-21","2026-11-22","2026-11-23","2026-11-24"],"weather_code":[80,61,77,3],"temperature_2m_max":[6.7,4.9,3.0,1.7],"temperature_2m_min":[0.6,2.1,-0.6,-1.0],"sunrise":["2026-11-21T07:38","2026-11-22T07:39","2026-11-23T07:41","2026-11-24T07:43"],"sunset":["2026-11-21T16:05","2026-11-22T16:04","2026-11-23T16:03","2026-11-24T16:02"]}}"#;

/// Renders every screen into PNG files without opening a window. The clock
/// is fixed so the output is reproducible.
fn screenshots(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut display = SimulatorDisplay::<Rgb565>::new(Size::new(SCREEN_W as u32, SCREEN_H as u32));
    let mut app = App::new(Settings::default(), None);
    let mut env = Env {
        now_ms: 0,
        local: Some(LocalTime {
            year: 2026,
            month: 11,
            day: 21,
            weekday: 5,
            hour: 17,
            minute: 42,
            second: 37,
        }),
        net: connected_net(),
    };
    let mut count = 0;
    let mut shot = |app: &mut App, env: &Env, name: &str| -> Result<(), String> {
        let Ok(()) = app.draw(&mut display, env);
        count += 1;
        let file = dir.join(format!("{count:02}-{name}.png"));
        display
            .to_rgb_output_image(&output_settings())
            .save_png(&file)
            .map_err(|e| format!("{}: {e}", file.display()))
    };
    // Lets animations settle, with an optional finger on the screen.
    fn run(app: &mut App, env: &mut Env, ms: u64, touch: Option<(i32, i32)>) {
        for _ in 0..ms / 30 {
            env.now_ms += 30;
            app.tick(env, touch.map(|(x, y)| to_raw(Point::new(x, y), true)));
        }
        app.take_effects();
    }
    let view = |app: &mut App, env: &Env, name: &str| {
        post(app, env, "/api/view", &format!(r#"{{"view":"{name}"}}"#))
    };

    shot(&mut app, &env, "calibrate")?;
    for (x, y) in [(30, 30), (450, 30), (450, 290), (30, 290)] {
        run(&mut app, &mut env, 300, Some((x, y)));
        run(&mut app, &mut env, 150, None);
    }
    // Long enough that the demo items' ages are not all "now".
    env.now_ms += 3_600_000;

    shot(&mut app, &env, "home-empty")?;
    demo(&mut app, &env)?;
    app.set_weather(SAMPLE_WEATHER, &env)?;
    shot(&mut app, &env, "home")?;
    for i in 0..4 {
        request(
            &mut app,
            &env,
            "PUT",
            &format!("/api/items/extra{i}"),
            r#"{"text":"Another notification","color":"grey"}"#,
        )?;
    }
    shot(&mut app, &env, "home-overflow")?;
    request(&mut app, &env, "DELETE", "/api/items", "")?;
    demo(&mut app, &env)?;

    view(&mut app, &env, "forecast")?;
    shot(&mut app, &env, "forecast")?;
    view(&mut app, &env, "calendar")?;
    shot(&mut app, &env, "calendar")?;

    view(&mut app, &env, "home")?;
    for (code, is_day, name) in [
        (0, 1, "clear-day"),
        (0, 0, "clear-night"),
        (3, 1, "cloudy"),
        (45, 1, "fog"),
        (63, 1, "rain"),
        (73, 0, "snow"),
        (95, 1, "thunder"),
    ] {
        let body = format!(
            r#"{{"current":{{"temperature_2m":-3.4,"weather_code":{code},"is_day":{is_day}}},"daily":{{"temperature_2m_max":[4.2],"temperature_2m_min":[-6.8]}}}}"#
        );
        app.set_weather(&body, &env)?;
        shot(&mut app, &env, &format!("weather-{name}"))?;
    }
    app.set_weather(SAMPLE_WEATHER, &env)?;

    // Long press opens the menu.
    run(&mut app, &mut env, 900, Some((240, 160)));
    run(&mut app, &mut env, 150, None);
    shot(&mut app, &env, "menu")?;
    // Tap "Close".
    run(&mut app, &mut env, 90, Some((240, 283)));
    run(&mut app, &mut env, 150, None);

    post(
        &mut app,
        &env,
        "/api/settings",
        r##"{"theme":"light","accent_color":"#E0457B"}"##,
    )?;
    for name in ["home", "forecast", "calendar"] {
        view(&mut app, &env, name)?;
        run(&mut app, &mut env, 1500, None);
        shot(&mut app, &env, &format!("light-{name}"))?;
    }

    env.net = setup_net();
    run(&mut app, &mut env, 60, None);
    shot(&mut app, &env, "wifi-setup")?;

    println!("{count} screenshots written to {}", dir.display());
    Ok(())
}
