//! Desktop simulator: the real UI in an SDL window and the real HTTP API on
//! localhost. Only the hardware drivers are replaced.
//!
//! ```text
//! cargo run -p simulator                      # window + http://127.0.0.1:8080
//! cargo run -p simulator -- --calibrate       # start with the calibration screen
//! cargo run -p simulator -- --setup           # start in WiFi setup mode
//! cargo run -p simulator -- --demo            # sample feed items and weather
//! cargo run -p simulator -- --screenshots DIR # render every screen to PNG, no window
//! cargo run -p simulator -- --board crowpanel-579  # another board (default freenove-35)
//! ```
//!
//! Boards without touch use the keyboard: the arrow keys are the rotary
//! switch, Enter presses it, Escape is the EXIT button and M the MENU button.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use chrono::{Datelike, Local, Timelike};
use embedded_graphics::{pixelcolor::Rgb565, prelude::*};
use embedded_graphics_simulator::{
    OutputSettings, OutputSettingsBuilder, SimulatorDisplay, SimulatorEvent, Window,
    sdl2::{Keycode, MouseButton},
};
use ui::{
    Action, App, Calibration, Effect, Env, Led, LocalTime, NetState, Profile, RawTouch, Settings,
    api,
    present::{Frame, FramePresenter, Refresh, RegionPresenter},
};

/// Prints a line like the firmware's log and keeps it for `/api/log`.
macro_rules! say {
    ($($arg:tt)*) => {{
        let line = format!($($arg)*);
        println!("{line}");
        ui::logbuf::push(&line, false);
    }};
}

/// Like [`say`], for warnings: also queued for `<prefix>/log`.
macro_rules! warn {
    ($($arg:tt)*) => {{
        let line = format!($($arg)*);
        eprintln!("{line}");
        ui::logbuf::push(&line, true);
    }};
}

const SETTINGS_FILE: &str = "sim-settings.json";
const CALIBRATION_FILE: &str = "sim-calibration.json";
const TICK: Duration = Duration::from_millis(30);

struct Shared {
    app: Mutex<App>,
    net: Mutex<NetState>,
    mqtt: Mutex<Option<bool>>,
    /// Screens to publish retained on MQTT: name and payload (`None` clears).
    outbox: Mutex<Vec<(String, Option<String>)>>,
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
            net: self
                .net
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            unix_s: Some(now.timestamp()),
            mqtt: *self.mqtt.lock().unwrap_or_else(PoisonError::into_inner),
            rssi: Some(-48),
            heap_free: None,
            firmware_slot: None,
        }
    }

    /// Carries out what the firmware would do with NVS, the speaker and `WiFi`.
    fn apply_effects(&self, app: &mut App) {
        for effect in app.take_effects() {
            match effect {
                Effect::SaveSettings => {
                    self.save(SETTINGS_FILE, &serde_json::json!(app.settings()));
                }
                Effect::SaveCalibration(c) => self.save(CALIBRATION_FILE, &serde_json::json!(c)),
                Effect::Beep(beep) => say!("[speaker] {beep:?}"),
                Effect::StartWifiSetup => {
                    say!("[wifi] credentials erased, setup access point opened");
                    *self.net.lock().unwrap_or_else(PoisonError::into_inner) = setup_net();
                }
                Effect::SetWifi { ssid, .. } => {
                    say!("[wifi] credentials for {ssid:?} saved, reconnecting");
                    *self.net.lock().unwrap_or_else(PoisonError::into_inner) = connected_net();
                }
                Effect::PublishScreen { name, payload } => {
                    self.outbox
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push((name, payload));
                }
                Effect::Restart => say!("[system] restart requested (ignored in the simulator)"),
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
        ssid: "Herald-A1B2".into(),
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

#[expect(clippy::struct_excessive_bools, reason = "one per command-line flag")]
struct Args {
    calibrate: bool,
    setup: bool,
    demo: bool,
    /// No window: the app runs and draws off-screen, for tests.
    headless: bool,
    /// The board to simulate.
    board: Profile,
    port: u16,
    screenshots: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        calibrate: false,
        setup: false,
        demo: false,
        headless: false,
        board: Profile::FREENOVE_35,
        port: 8080,
        screenshots: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--calibrate" => args.calibrate = true,
            "--setup" => args.setup = true,
            "--demo" => args.demo = true,
            "--headless" => args.headless = true,
            "--board" => {
                let value = it.next().ok_or("--board needs a name")?;
                args.board = Profile::by_name(&value).ok_or_else(|| {
                    let names: Vec<_> = Profile::ALL.iter().map(|p| p.name).collect();
                    format!("unknown board {value}; known: {}", names.join(", "))
                })?;
            }
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
                "{e}\nusage: simulator [--calibrate] [--setup] [--demo] [--headless] [--board NAME] [--port N] [--screenshots DIR]"
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

    // Headless runs (tests) start from defaults and store nothing.
    let storage = (!args.headless).then(|| PathBuf::from("."));
    let settings: Settings = storage
        .as_deref()
        .and_then(|dir| load(dir, SETTINGS_FILE))
        .filter(|s: &Settings| s.validate().is_ok())
        .unwrap_or_default();
    // The mouse already reports screen coordinates.
    let board = args.board;
    let calibration = (!args.calibrate).then(|| Calibration::identity(board.width, board.height));
    let shared = Arc::new(Shared {
        app: Mutex::new(App::new(board, settings, calibration)),
        net: Mutex::new(if args.setup {
            setup_net()
        } else {
            connected_net()
        }),
        mqtt: Mutex::new(None),
        outbox: Mutex::new(Vec::new()),
        start: Instant::now(),
        storage,
    });

    if args.demo {
        let env = shared.env();
        let mut app = shared.app.lock().unwrap_or_else(PoisonError::into_inner);
        if let Err(e) = demo(&mut app, &env) {
            eprintln!("demo data rejected: {e}");
        }
    }
    serve_http(Arc::clone(&shared), args.port);
    run_mqtt(Arc::clone(&shared));
    if args.headless {
        run_headless(&shared);
    } else if board.frame {
        run_frame(&shared);
    } else {
        run_window(&shared, args.calibrate);
    }
}

/// The same requests a phone, Home Assistant or the Claude Code hook would
/// send, so the views have something to show.
fn demo(app: &mut App, env: &Env) -> Result<(), String> {
    request(app, env, "PUT", "/api/weather", SAMPLE_WEATHER)?;
    for (id, body) in DEMO_ITEMS {
        request(app, env, "PUT", &format!("/api/items/{id}"), body)?;
    }
    for (name, screen) in DEMO_SCREENS {
        request(app, env, "PUT", &format!("/api/screens/{name}"), screen)?;
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
            if_none_match: None,
        },
    );
    app.take_effects();
    (r.status == 200)
        .then_some(())
        .ok_or_else(|| format!("{method} {path}: {}", r.text()))
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
    let size = Profile::FREENOVE_35;
    let (x, y) = (p.x.clamp(0, size.width - 1), p.y.clamp(0, size.height - 1));
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

/// The app loop without a window: ticks and draws off-screen.
fn run_headless(shared: &Shared) {
    let mut display = SimulatorDisplay::<Rgb565>::new(ui::views::area().size);
    let mut present = RegionPresenter::default();
    loop {
        let env = shared.env();
        {
            let mut app = shared.app.lock().unwrap_or_else(PoisonError::into_inner);
            app.tick(&env);
            shared.apply_effects(&mut app);
            let Ok(()) = present.draw(&mut app, &mut display, &env);
            drop(app);
        }
        thread::sleep(TICK);
    }
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "of the window events only the left mouse button and quitting matter"
)]
fn run_window(shared: &Shared, fake_adc: bool) {
    let size = ui::views::area().size;
    let mut display = SimulatorDisplay::<Rgb565>::new(size);
    let dark = SimulatorDisplay::<Rgb565>::new(size);
    let mut window = Window::new("Herald simulator", &output_settings());
    let mut touch: Option<Point> = None;
    let mut led = Led::Off;
    let mut present = RegionPresenter::default();

    loop {
        let frame_start = Instant::now();
        let env = shared.env();
        let asleep = {
            let mut app = shared.app.lock().unwrap_or_else(PoisonError::into_inner);
            let raw = touch.map(|p| to_raw(p, fake_adc));
            app.touch(raw, &env);
            app.tick(&env);
            shared.apply_effects(&mut app);
            let Ok(()) = present.draw(&mut app, &mut display, &env);
            let now_led = app.led();
            if now_led != led {
                led = now_led;
                say!("[led] {led:?}");
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

/// The e-paper board: redraws once a second like the firmware, and logs
/// each refresh the panel would do.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "of the window events only a few keys and quitting matter"
)]
fn run_frame(shared: &Shared) {
    let size = {
        let app = shared.app.lock().unwrap_or_else(PoisonError::into_inner);
        app.layout().area().size
    };
    let mut display = SimulatorDisplay::<Rgb565>::new(size);
    let mut window = Window::new("Herald e-paper", &output_settings());
    // Enough rows for one pass in either orientation.
    let mut present = FramePresenter::new(size.width.max(size.height) as usize);
    let mut next_render = Instant::now();
    loop {
        let env = shared.env();
        if Instant::now() >= next_render {
            let next = {
                let mut app = shared.app.lock().unwrap_or_else(PoisonError::into_inner);
                app.tick(&env);
                shared.apply_effects(&mut app);
                let next = present.next(&mut app, &env);
                drop(app);
                next
            };
            if let Some((frame, kind)) = next {
                say!(
                    "[epaper] {} refresh",
                    if kind == Refresh::Full {
                        "full"
                    } else {
                        "partial"
                    }
                );
                let frame_size = Size::new(frame.width() as u32, frame.height() as u32);
                if frame_size != display.size() {
                    // Turned: a window of the new shape.
                    display = SimulatorDisplay::new(frame_size);
                    window = Window::new("Herald e-paper", &output_settings());
                }
                show_frame(&mut display, &frame);
                shared
                    .app
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .set_shown_frame(frame);
            }
            next_render = Instant::now() + Duration::from_secs(1);
        }
        window.update(&display);
        for event in window.events() {
            let action = match event {
                SimulatorEvent::Quit => return,
                SimulatorEvent::KeyDown {
                    keycode,
                    repeat: false,
                    ..
                } => match keycode {
                    // As on the board: the rotary switch, EXIT and MENU.
                    Keycode::Up | Keycode::Left => Action::Previous,
                    Keycode::Down | Keycode::Right => Action::Next,
                    Keycode::Return | Keycode::Space | Keycode::Escape => Action::Home,
                    Keycode::M => Action::Refresh,
                    _ => continue,
                },
                _ => continue,
            };
            let env = shared.env();
            shared
                .app
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .act(action, &env);
            next_render = Instant::now();
        }
        thread::sleep(TICK);
    }
}

fn show_frame(display: &mut SimulatorDisplay<Rgb565>, frame: &Frame) {
    for y in 0..frame.height() {
        for x in 0..frame.width() {
            let color = if frame.black(x, y) {
                Rgb565::new(4, 8, 4)
            } else {
                Rgb565::new(28, 57, 27)
            };
            let Ok(()) = Pixel(Point::new(x, y), color).draw(display);
        }
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
            let if_none_match = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("If-None-Match"))
                .map(|h| h.value.as_str().to_owned());
            let method = request.method().as_str().to_owned();
            let path = request.url().to_owned();

            let response = {
                let env = shared.env();
                let mut app = shared.app.lock().unwrap_or_else(PoisonError::into_inner);
                let response = api::handle(
                    &mut app,
                    &env,
                    &api::Request {
                        method: &method,
                        path: &path,
                        body: &body,
                        token: token.as_deref(),
                        if_none_match: if_none_match.as_deref(),
                    },
                );
                shared.apply_effects(&mut app);
                drop(app);
                response
            };
            let mut reply = tiny_http::Response::from_data(response.body.into_owned())
                .with_status_code(response.status);
            let headers = std::iter::once(("Content-Type", response.content_type.to_owned()))
                .chain(response.headers);
            for (name, value) in headers {
                if let Ok(h) = tiny_http::Header::from_bytes(name, value) {
                    reply.add_header(h);
                }
            }
            if let Err(e) = request.respond(reply) {
                eprintln!("HTTP response failed: {e}");
            }
        }
    });
}

// ------------------------------------------------------------------ MQTT

/// One broker connection. Its thread ends once `stop` is set.
struct MqttConnection {
    config: ui::mqtt::Config,
    client: rumqttc::Client,
    connected: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl MqttConnection {
    fn open(shared: &Arc<Shared>, config: ui::mqtt::Config) -> Self {
        use rumqttc::{Event, LastWill, MqttOptions, Packet, QoS};

        let mut options = MqttOptions::new("herald-sim", config.host.clone(), config.port);
        options.set_keep_alive(Duration::from_secs(30));
        options.set_last_will(LastWill::new(
            config.status_topic(),
            ui::mqtt::OFFLINE,
            QoS::AtLeastOnce,
            true,
        ));
        if !config.user.is_empty() {
            options.set_credentials(config.user.clone(), config.password.clone());
        }
        let (client, mut connection) = rumqttc::Client::new(options, 32);
        let connected = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));

        let (shared, flag, stopped, subscriber) = (
            Arc::clone(shared),
            Arc::clone(&connected),
            Arc::clone(&stop),
            client.clone(),
        );
        let transport = config.transport();
        let topics = config.subscriptions();
        let status = config.status_topic();
        let error_topic = config.error_topic();
        let url = config.url();
        thread::spawn(move || {
            for event in connection.iter() {
                if stopped.load(Ordering::Relaxed) {
                    break;
                }
                match event {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        say!("[mqtt] connected to {url}");
                        flag.store(true, Ordering::Relaxed);
                        for topic in &topics {
                            let _ = subscriber.try_subscribe(topic.as_str(), QoS::AtLeastOnce);
                        }
                        let _ = subscriber.try_publish(
                            status.as_str(),
                            QoS::AtLeastOnce,
                            true,
                            ui::mqtt::ONLINE,
                        );
                    }
                    Ok(Event::Incoming(Packet::Publish(p))) => {
                        let env = shared.env();
                        let mut app = shared.app.lock().unwrap_or_else(PoisonError::into_inner);
                        let message = ui::mqtt::Message {
                            topic: &p.topic,
                            payload: &p.payload,
                        };
                        let result = ui::command::handle(&transport, &mut app, &env, &message);
                        shared.apply_effects(&mut app);
                        drop(app);
                        match result {
                            Ok(_) => say!("[mqtt] {}", p.topic),
                            Err(e) => {
                                warn!("[mqtt] {}: {}", p.topic, e.message);
                                let report = ui::mqtt::error_payload(&message, &e);
                                let _ = subscriber.try_publish(
                                    error_topic.as_str(),
                                    QoS::AtMostOnce,
                                    false,
                                    report,
                                );
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        if flag.swap(false, Ordering::Relaxed) {
                            warn!("[mqtt] disconnected: {e}");
                        }
                        // The next iteration reconnects.
                        thread::sleep(Duration::from_secs(2));
                    }
                }
            }
        });
        Self {
            config,
            client,
            connected,
            stop,
        }
    }

    fn close(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.client.disconnect();
    }
}

/// Follows the MQTT settings, like the firmware's network thread does, and
/// publishes the state.
fn run_mqtt(shared: Arc<Shared>) {
    thread::spawn(move || {
        let mut current: Option<MqttConnection> = None;
        loop {
            let wanted = ui::mqtt::Config::from_settings(
                shared
                    .app
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .settings(),
            );
            if current.as_ref().map(|c| &c.config) != wanted.as_ref() {
                if let Some(old) = current.take() {
                    old.close();
                }
                current = wanted.map(|config| MqttConnection::open(&shared, config));
            }
            let connected = current
                .as_ref()
                .map(|c| c.connected.load(Ordering::Relaxed));
            *shared.mqtt.lock().unwrap_or_else(PoisonError::into_inner) = connected;
            if let (Some(c), Some(true)) = (&current, connected) {
                let qos = rumqttc::QoS::AtLeastOnce;
                let outbox = std::mem::take(
                    &mut *shared.outbox.lock().unwrap_or_else(PoisonError::into_inner),
                );
                for (name, payload) in outbox {
                    let topic = c.config.screen_topic(&name);
                    let _ = c
                        .client
                        .try_publish(topic, qos, true, payload.unwrap_or_default());
                }
                for line in ui::logbuf::take_pending() {
                    let _ = c.client.try_publish(c.config.log_topic(), qos, false, line);
                }
            }
            thread::sleep(Duration::from_secs(1));
        }
    });
}

// ------------------------------------------------------------ screenshots

/// Custom screens as Home Assistant might send them, for the screenshots.
const DEMO_SCREENS: [(&str, &str); 2] = [
    (
        "energy",
        r#"{"title":"Energy","widgets":[
            {"type":"value","label":"Solar now","value":3.2,"unit":"kW","color":"yellow"},
            {"type":"gauge","label":"Battery","value":64,"unit":"%","zones":[{"to":20,"color":"red"},{"to":60,"color":"yellow"},{"to":100,"color":"green"}]},
            {"type":"chart","label":"Today","values":[0,0,0.2,0.9,1.8,2.6,3.2,3.0,2.4,1.1,0.3],"unit":" kW","color":"yellow"},
            {"type":"list","label":"Usage","rows":[{"label":"Heat pump","value":"1.4 kW"},{"label":"Car","value":"0 kW"},{"label":"House","value":"0.6 kW"}]},
            {"type":"progress","label":"Car charged","value":82,"color":"green"},
            {"type":"countdown","label":"Cheap power in","until":1795371000}
        ]}"#,
    ),
    (
        "house",
        r##"{"title":"At home","widgets":[
            {"type":"icon","icon":"window","label":"Kitchen","value":"open","color":"red"},
            {"type":"icon","icon":"door","label":"Front door","value":"locked","color":"green"},
            {"type":"grid","tiles":[{"label":"CO2","value":812,"unit":" ppm","color":"yellow"},{"label":"Living","value":21.5,"unit":"°"},{"label":"Bath","value":23.1,"unit":"°"},{"label":"Outside","value":4,"unit":"°","color":"#4A9BFF"}]},
            {"type":"value","label":"Washing machine","value":"0:42","digits":true}
        ]}"##,
    ),
];

/// Feed texts as long as Home Assistant entity names get.
fn long_items(app: &mut App, env: &Env) -> Result<(), String> {
    for (id, text) in [
        ("door.terrace", "Living room terrace door open"),
        ("window.bedroom", "Upstairs bedroom window (left) tilted"),
        ("door.garage", "Garage door"),
        (
            "lock.garden",
            "Basement laundry room garden door unlocked since noon",
        ),
    ] {
        let body = format!(r#"{{"text":"{text}","color":"yellow","age":4000}}"#);
        request(app, env, "PUT", &format!("/api/items/{id}"), &body)?;
    }
    Ok(())
}

/// Weather as Home Assistant would push it (Berlin, 2026-11-21), for the
/// demo data and the screenshots.
const SAMPLE_WEATHER: &str = r#"{"temp":6.4,"condition":"cloudy","is_day":false,"sunrise":"07:38","sunset":"16:05","hourly":[{"hour":17,"temp":6.4,"rain":23,"condition":"cloudy","is_day":false},{"hour":18,"temp":6.2,"rain":40,"condition":"cloudy","is_day":false},{"hour":19,"temp":5.9,"rain":35,"condition":"rainy","is_day":false},{"hour":20,"temp":5.7,"rain":18,"condition":"rainy","is_day":false},{"hour":21,"temp":5.5,"rain":13,"condition":"rainy","is_day":false},{"hour":22,"temp":5.3,"rain":10,"condition":"cloudy","is_day":false},{"hour":23,"temp":5.2,"rain":20,"condition":"cloudy","is_day":false},{"hour":0,"temp":4.9,"rain":33,"condition":"cloudy","is_day":false},{"hour":1,"temp":4.8,"rain":45,"condition":"rainy","is_day":false},{"hour":2,"temp":4.8,"rain":45,"condition":"rainy","is_day":false},{"hour":3,"temp":4.7,"rain":28,"condition":"cloudy","is_day":false},{"hour":4,"temp":4.5,"rain":13,"condition":"partlycloudy","is_day":false}],"daily":[{"date":"2026-11-21","min":0.6,"max":6.7,"condition":"rainy"},{"date":"2026-11-22","min":2.1,"max":4.9,"condition":"rainy"},{"date":"2026-11-23","min":-0.6,"max":3.0,"condition":"snowy"},{"date":"2026-11-24","min":-1.0,"max":1.7,"condition":"cloudy"}]}"#;

/// The e-paper layout: the dashboard, custom screens and setup.
fn epaper_screenshots(dir: &Path, env: &mut Env) -> Result<usize, String> {
    let settings = Settings {
        theme: ui::model::ThemeKind::Light,
        ..Settings::default()
    };
    let app = &mut App::new(Profile::CROWPANEL_579, settings, None);
    let size = app.layout().area().size;
    let mut display = SimulatorDisplay::<Rgb565>::new(size);
    let mut present = FramePresenter::new(size.height as usize);
    let mut count = 0;
    let mut shot = |app: &App, env: &Env, name: &str| -> Result<(), String> {
        show_frame(&mut display, &present.render(app, env));
        count += 1;
        let file = dir.join(format!("epaper-{count:02}-{name}.png"));
        display
            .to_rgb_output_image(&output_settings())
            .save_png(&file)
            .map_err(|e| format!("{}: {e}", file.display()))
    };
    env.net = connected_net();
    run(app, env, 60, None);
    request(app, env, "DELETE", "/api/items", "")?;
    app.clear_weather();
    post(app, env, "/api/view", r#"{"view":"home"}"#)?;
    shot(app, env, "empty")?;
    demo(app, env)?;
    post(app, env, "/api/view", r#"{"view":"home"}"#)?;
    shot(app, env, "dashboard")?;
    for i in 0..6 {
        request(
            app,
            env,
            "PUT",
            &format!("/api/items/extra{i}"),
            r#"{"text":"Another notification","color":"grey"}"#,
        )?;
    }
    shot(app, env, "overflow")?;
    request(app, env, "DELETE", "/api/items", "")?;
    long_items(app, env)?;
    shot(app, env, "long")?;
    for (name, _) in DEMO_SCREENS {
        post(app, env, "/api/view", &format!(r#"{{"view":"{name}"}}"#))?;
        shot(app, env, &format!("screen-{name}"))?;
    }
    env.net = setup_net();
    run(app, env, 60, None);
    shot(app, env, "setup")?;

    // Landscape: the wide layout.
    post(app, env, "/api/settings", r#"{"orientation":"landscape"}"#)?;
    let size = app.layout().area().size;
    let mut display = SimulatorDisplay::<Rgb565>::new(size);
    let mut present = FramePresenter::new(size.width as usize);
    let mut shot = |app: &App, env: &Env, name: &str| -> Result<(), String> {
        show_frame(&mut display, &present.render(app, env));
        count += 1;
        let file = dir.join(format!("epaper-{count:02}-wide-{name}.png"));
        display
            .to_rgb_output_image(&output_settings())
            .save_png(&file)
            .map_err(|e| format!("{}: {e}", file.display()))
    };
    shot(app, env, "setup")?;
    env.net = connected_net();
    run(app, env, 60, None);
    post(app, env, "/api/view", r#"{"view":"home"}"#)?;
    shot(app, env, "dashboard")?;
    for (name, _) in DEMO_SCREENS {
        post(app, env, "/api/view", &format!(r#"{{"view":"{name}"}}"#))?;
        shot(app, env, &format!("screen-{name}"))?;
    }
    post(app, env, "/api/settings", r#"{"language":"de"}"#)?;
    post(app, env, "/api/view", r#"{"view":"home"}"#)?;
    shot(app, env, "de-dashboard")?;
    Ok(count)
}

/// Lets animations settle, with an optional finger on the screen.
fn run(app: &mut App, env: &mut Env, ms: u64, touch: Option<(i32, i32)>) {
    for _ in 0..ms / 30 {
        env.now_ms += 30;
        app.touch(touch.map(|(x, y)| to_raw(Point::new(x, y), true)), env);
        app.tick(env);
    }
    app.take_effects();
}

/// Renders every screen into PNG files without opening a window. The clock
/// is fixed so the output is reproducible.
#[expect(
    clippy::too_many_lines,
    reason = "one fixed sequence of shots, read top to bottom"
)]
fn screenshots(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut display = SimulatorDisplay::<Rgb565>::new(ui::views::area().size);
    let mut app = App::new(Profile::FREENOVE_35, Settings::default(), None);
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
        // 2026-11-21 17:42:37 in Berlin.
        unix_s: Some(1_795_365_757),
        ..Env::default()
    };
    let mut count = 0;
    let mut present = RegionPresenter::default();
    let mut shot = |app: &mut App, env: &Env, name: &str| -> Result<(), String> {
        let Ok(()) = present.draw(app, &mut display, env);
        count += 1;
        let file = dir.join(format!("{count:02}-{name}.png"));
        display
            .to_rgb_output_image(&output_settings())
            .save_png(&file)
            .map_err(|e| format!("{}: {e}", file.display()))
    };
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
    long_items(&mut app, &env)?;
    shot(&mut app, &env, "home-long")?;
    request(&mut app, &env, "DELETE", "/api/items", "")?;
    demo(&mut app, &env)?;

    view(&mut app, &env, "forecast")?;
    shot(&mut app, &env, "forecast")?;
    view(&mut app, &env, "calendar")?;
    shot(&mut app, &env, "calendar")?;

    for (name, screen) in DEMO_SCREENS {
        request(
            &mut app,
            &env,
            "PUT",
            &format!("/api/screens/{name}"),
            screen,
        )?;
        view(&mut app, &env, name)?;
        shot(&mut app, &env, &format!("screen-{name}"))?;
        request(
            &mut app,
            &env,
            "DELETE",
            &format!("/api/screens/{name}"),
            "",
        )?;
    }

    view(&mut app, &env, "home")?;
    for (condition, is_day, name) in [
        ("sunny", true, "clear-day"),
        ("clear-night", false, "clear-night"),
        ("cloudy", true, "cloudy"),
        ("fog", true, "fog"),
        ("rainy", true, "rain"),
        ("snowy", false, "snow"),
        ("lightning", true, "thunder"),
    ] {
        let body = format!(
            r#"{{"temp":-3.4,"condition":"{condition}","is_day":{is_day},"min":-6.8,"max":4.2}}"#
        );
        request(&mut app, &env, "PUT", "/api/weather", &body)?;
        shot(&mut app, &env, &format!("weather-{name}"))?;
    }
    request(&mut app, &env, "PUT", "/api/weather", SAMPLE_WEATHER)?;

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
    post(&mut app, &env, "/api/settings", r#"{"language":"de"}"#)?;
    for name in ["home", "forecast", "calendar"] {
        view(&mut app, &env, name)?;
        run(&mut app, &mut env, 1500, None);
        shot(&mut app, &env, &format!("de-{name}"))?;
    }
    post(&mut app, &env, "/api/settings", r#"{"language":"en"}"#)?;

    env.net = setup_net();
    run(&mut app, &mut env, 60, None);
    shot(&mut app, &env, "wifi-setup")?;

    count += epaper_screenshots(dir, &mut env)?;

    println!("{count} screenshots written to {}", dir.display());
    Ok(())
}
