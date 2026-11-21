//! The application: owns all state, reacts to touch and API calls, and
//! repaints only the screen regions whose content changed.

use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};

use crate::{
    feed::{Feed, Item},
    gesture::{Dir, Gesture, Recognizer},
    gfx::{self, Canvas},
    model::{
        screen, Beep, Calibration, Effect, Env, ItemColor, Led, NetState, RawTouch, Settings,
        ViewId,
    },
    theme::Theme,
    views::{self, Region},
    weather::{self, Weather},
};

/// After a touch, rotation waits this long before it moves on.
pub const ROTATE_TOUCH_PAUSE_MS: u64 = 60_000;

/// What currently fills the display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Screen {
    Calibrate,
    WifiSetup,
    Menu,
    View(ViewId),
}

pub struct App {
    pub(crate) settings: Settings,
    pub(crate) calibration: Option<Calibration>,
    pub(crate) view: ViewId,
    pub(crate) menu_open: bool,
    pub(crate) menu_confirm_wifi: bool,
    pub(crate) calibrate: Option<views::setup::CalibrateState>,
    pub(crate) setup_dismissed: bool,
    pub(crate) feed: Feed,
    pub(crate) weather: Option<Weather>,
    next_weather_ms: u64,
    gesture: Recognizer,
    last_activity_ms: u64,
    /// When the view last changed, for rotation.
    last_switch_ms: u64,
    pub(crate) asleep: bool,
    /// The touch that woke the display must not trigger anything else.
    swallow_touch: bool,
    effects: Vec<Effect>,
    buf: Vec<Rgb565>,
    cache: Vec<u64>,
    cache_screen: Option<(Screen, i64)>,
}

impl App {
    /// `calibration` is `None` on first boot, which starts with the touch
    /// calibration screen. So does a calibration made for another layout.
    pub fn new(settings: Settings, calibration: Option<Calibration>) -> Self {
        let calibration = calibration.filter(Calibration::fits_screen);
        Self {
            settings,
            calibrate: calibration
                .is_none()
                .then(views::setup::CalibrateState::default),
            calibration,
            view: ViewId::Home,
            menu_open: false,
            menu_confirm_wifi: false,
            setup_dismissed: false,
            feed: Feed::default(),
            weather: None,
            next_weather_ms: 0,
            gesture: Recognizer::new(),
            last_activity_ms: 0,
            last_switch_ms: 0,
            asleep: false,
            swallow_touch: false,
            effects: Vec::new(),
            buf: vec![Rgb565::BLACK; gfx::BAND_PIXELS],
            cache: Vec::new(),
            cache_screen: None,
        }
    }

    // ------------------------------------------------------------ accessors

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn view(&self) -> ViewId {
        self.view
    }

    pub fn feed(&self) -> &[Item] {
        self.feed.items()
    }

    pub fn weather(&self) -> Option<&Weather> {
        self.weather.as_ref()
    }

    pub fn is_asleep(&self) -> bool {
        self.asleep
    }

    /// Side effects for the platform to carry out. Call after every tick and
    /// every API request.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// Backlight level in percent.
    pub fn backlight(&self) -> u8 {
        if self.asleep {
            0
        } else {
            self.settings.brightness
        }
    }

    /// A red or yellow item at the top of the feed lights the LED in its
    /// colour.
    pub fn led(&self) -> Led {
        match self.feed.top().map(|i| i.color) {
            Some(ItemColor::Red) => Led::Red,
            Some(ItemColor::Yellow) => Led::Yellow,
            _ => Led::Off,
        }
    }

    pub(crate) fn theme(&self) -> Theme {
        Theme::from_settings(&self.settings)
    }

    // ------------------------------------------------------------ commands

    pub fn wake(&mut self, now_ms: u64) {
        self.asleep = false;
        self.last_activity_ms = now_ms;
    }

    pub fn set_view(&mut self, view: ViewId, now_ms: u64) {
        self.view = view;
        self.menu_open = false;
        self.last_switch_ms = now_ms;
    }

    /// Replaces the settings after validating them.
    pub fn update_settings(&mut self, new: Settings) -> Result<(), String> {
        new.validate()?;
        let location_changed = (new.latitude, new.longitude, &new.location)
            != (
                self.settings.latitude,
                self.settings.longitude,
                &self.settings.location,
            );
        if location_changed {
            self.weather = None;
            self.next_weather_ms = 0;
        }
        if new != self.settings {
            self.settings = new;
            self.effects.push(Effect::SaveSettings);
        }
        Ok(())
    }

    /// Adds or replaces a feed item; see [`Feed::upsert`].
    pub fn put_item(&mut self, item: Item, explicit_since: bool) -> Result<(), String> {
        self.feed.upsert(item, explicit_since)
    }

    pub fn remove_item(&mut self, id: &str) -> bool {
        self.feed.remove(id)
    }

    pub fn clear_items(&mut self) {
        self.feed.clear();
    }

    /// Brings the home view up so a new item is seen.
    pub fn alert(&mut self, beep: bool, now_ms: u64) {
        self.wake(now_ms);
        if self.calibrate.is_none() {
            self.set_view(ViewId::Home, now_ms);
        }
        if beep {
            self.effects.push(Effect::Beep(Beep::Notice));
        }
    }

    pub(crate) fn push_effect(&mut self, effect: Effect) {
        self.effects.push(effect);
    }

    // ------------------------------------------------------------- weather

    /// Returns the URL to fetch when a weather update is due. The platform
    /// reports the outcome through [`App::set_weather`].
    pub fn weather_due(&mut self, env: &Env) -> Option<String> {
        if !self.settings.has_location()
            || !matches!(env.net, NetState::Connected { .. })
            || env.now_ms < self.next_weather_ms
        {
            return None;
        }
        // Assume failure until told otherwise, so a hung request is retried.
        self.next_weather_ms = env.now_ms + weather::RETRY_MS;
        Some(weather::url(&self.settings))
    }

    /// `body` is the HTTP response body of the weather request.
    pub fn set_weather(&mut self, body: &str, env: &Env) -> Result<(), String> {
        let mut w = weather::parse(body)?;
        w.updated = env.local.map(|t| (t.hour, t.minute));
        self.weather = Some(w);
        self.next_weather_ms = env.now_ms + weather::REFRESH_MS;
        Ok(())
    }

    // ---------------------------------------------------------------- tick

    /// Advances the state by one step. `raw` is the current touch sample.
    pub fn tick(&mut self, env: &Env, raw: Option<RawTouch>) {
        let now = env.now_ms;

        if let Some(state) = self.calibrate.as_mut() {
            if let Some(calibration) = state.feed(raw) {
                self.calibration = Some(calibration);
                self.calibrate = None;
                self.effects.push(Effect::SaveCalibration(calibration));
                self.gesture.reset();
                self.swallow_touch = true;
                self.last_activity_ms = now;
            }
            return;
        }

        let touch = match (raw, self.calibration.as_ref()) {
            (Some(raw), Some(cal)) => Some(cal.map(raw)),
            _ => None,
        };
        if touch.is_some() {
            if self.asleep {
                self.asleep = false;
                self.swallow_touch = true;
            }
            self.last_activity_ms = now;
        }
        if self.swallow_touch {
            self.gesture.reset();
            if touch.is_none() {
                self.swallow_touch = false;
            }
        } else if let Some(gesture) = self.gesture.update(touch, now) {
            self.on_gesture(gesture, env);
        }

        self.feed.expire(now);
        if !matches!(env.net, NetState::Setup { .. }) {
            self.setup_dismissed = false;
        }

        if !self.asleep
            && self.settings.sleep_secs > 0
            && now.saturating_sub(self.last_activity_ms)
                >= u64::from(self.settings.sleep_secs) * 1000
        {
            self.asleep = true;
            self.menu_open = false;
        }
        self.rotate(env);
    }

    /// Moves to the next view of the rotation when it is time to.
    fn rotate(&mut self, env: &Env) {
        let s = &self.settings;
        let now = env.now_ms;
        let showing_view = matches!(self.screen(env), Screen::View(_));
        if !s.rotate
            || self.asleep
            || !showing_view
            || self.gesture.is_pressed()
            || now.saturating_sub(self.last_activity_ms) < ROTATE_TOUCH_PAUSE_MS
            || now.saturating_sub(self.last_switch_ms) < u64::from(s.rotate_secs) * 1000
            || !s.in_rotate_window(env.local.as_ref())
        {
            return;
        }
        let views = &s.rotate_views;
        let next = match views.iter().position(|v| *v == self.view) {
            Some(i) => views[(i + 1) % views.len()],
            None => views[0],
        };
        self.view = next;
        self.last_switch_ms = now;
    }

    pub(crate) fn screen(&self, env: &Env) -> Screen {
        if self.calibrate.is_some() {
            Screen::Calibrate
        } else if self.menu_open {
            Screen::Menu
        } else if matches!(env.net, NetState::Setup { .. }) && !self.setup_dismissed {
            Screen::WifiSetup
        } else {
            Screen::View(self.view)
        }
    }

    fn on_gesture(&mut self, gesture: Gesture, env: &Env) {
        match self.screen(env) {
            Screen::Calibrate => {}
            Screen::WifiSetup => match gesture {
                Gesture::Tap(_) => self.setup_dismissed = true,
                Gesture::LongPress(_) => self.menu_open = true,
                Gesture::Swipe(_) => {}
            },
            Screen::Menu => {
                if let Gesture::Tap(p) = gesture {
                    views::menu::tap(self, p);
                }
            }
            Screen::View(_) => match gesture {
                Gesture::LongPress(_) => {
                    self.menu_open = true;
                    self.menu_confirm_wifi = false;
                }
                Gesture::Swipe(Dir::Left) => self.step_view(1, env.now_ms),
                Gesture::Swipe(Dir::Right) => self.step_view(ViewId::ALL.len() - 1, env.now_ms),
                Gesture::Swipe(_) | Gesture::Tap(_) => {}
            },
        }
    }

    fn step_view(&mut self, by: usize, now_ms: u64) {
        let view = ViewId::ALL[(self.view.index() + by) % ViewId::ALL.len()];
        self.set_view(view, now_ms);
    }

    pub(crate) fn start_calibration(&mut self) {
        self.menu_open = false;
        self.calibrate = Some(views::setup::CalibrateState::default());
    }

    // ---------------------------------------------------------------- draw

    fn regions(&self, screen: Screen, env: &Env) -> Vec<Region> {
        match screen {
            Screen::Calibrate => views::setup::calibrate_regions(self),
            Screen::WifiSetup => views::setup::wifi_regions(env),
            Screen::Menu => views::menu::regions(self, env),
            Screen::View(view) => {
                let mut regions = match view {
                    ViewId::Home => views::home::regions(self, env),
                    ViewId::Forecast => views::forecast::regions(self),
                    ViewId::Calendar => views::calendar::regions(self, env),
                };
                regions.push(views::nav_region(view));
                regions
            }
        }
    }

    fn paint(&self, screen: Screen, c: &mut Canvas<'_>, env: &Env, theme: &Theme) {
        gfx::fill(c, self::screen(), theme.bg);
        match screen {
            Screen::Calibrate => views::setup::paint_calibrate(self, c, theme),
            Screen::WifiSetup => views::setup::paint_wifi(c, env, theme),
            Screen::Menu => views::menu::paint(self, c, env, theme),
            Screen::View(view) => {
                match view {
                    ViewId::Home => views::home::paint(self, c, env, theme),
                    ViewId::Forecast => views::forecast::paint(self, c, theme),
                    ViewId::Calendar => views::calendar::paint(self, c, env, theme),
                }
                views::paint_nav(c, view, theme);
            }
        }
    }

    /// Makes the next [`App::draw`] repaint the whole screen, e.g. after the
    /// display was re-initialised.
    pub fn invalidate(&mut self) {
        self.cache_screen = None;
    }

    /// Repaints whatever changed since the last call. The first call, a
    /// change of screen and a change of theme repaint everything.
    pub fn draw<D>(&mut self, display: &mut D, env: &Env) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        if self.asleep {
            return Ok(());
        }
        let screen = self.screen(env);
        let theme = self.theme();
        let regions = self.regions(screen, env);
        let cache_id = Some((screen, theme.key()));
        let full = self.cache_screen != cache_id || self.cache.len() != regions.len();

        let mut buf = std::mem::take(&mut self.buf);
        let mut paint = |rect: Rectangle| {
            gfx::render(display, &mut buf, rect, |c| {
                self.paint(screen, c, env, &theme)
            })
        };
        let result = if full {
            paint(self::screen())
        } else {
            regions
                .iter()
                .zip(&self.cache)
                .filter(|(region, cached)| region.key != **cached)
                .try_for_each(|(region, _)| paint(region.rect))
        };
        self.buf = buf;
        result?;
        self.cache = regions.iter().map(|r| r.key).collect();
        self.cache_screen = cache_id;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::LocalTime;

    fn env_at(now_ms: u64, hour: u8) -> Env {
        Env {
            now_ms,
            local: Some(LocalTime {
                year: 2026,
                month: 11,
                day: 21,
                weekday: 5,
                hour,
                minute: 0,
                second: 0,
            }),
            net: NetState::Offline,
            ..Env::default()
        }
    }

    fn rotating(views: Vec<ViewId>) -> App {
        let settings = Settings {
            rotate: true,
            rotate_secs: 10,
            rotate_views: views,
            sleep_secs: 0,
            ..Settings::default()
        };
        App::new(settings, Some(Calibration::identity()))
    }

    /// Ticks every 30 ms from `from` to `to` and records each view change.
    fn run(app: &mut App, from: u64, to: u64, hour: u8) -> Vec<(u64, ViewId)> {
        let mut seen = Vec::new();
        let mut t = from;
        while t < to {
            let before = app.view();
            app.tick(&env_at(t, hour), None);
            if app.view() != before {
                seen.push((t / 1000, app.view()));
            }
            t += 30;
        }
        seen
    }

    #[test]
    fn rotation_follows_the_list() {
        let mut a = rotating(vec![ViewId::Home, ViewId::Calendar, ViewId::Forecast]);
        // Nothing happens during the first minute (counts as recent touch).
        let seen = run(&mut a, 0, 95_000, 12);
        assert_eq!(
            seen,
            [
                (60, ViewId::Calendar),
                (70, ViewId::Forecast),
                (80, ViewId::Home),
                (90, ViewId::Calendar),
            ]
        );
    }

    #[test]
    fn rotation_from_a_view_outside_the_list_starts_at_the_beginning() {
        let mut a = rotating(vec![ViewId::Calendar, ViewId::Forecast]);
        assert_eq!(a.view(), ViewId::Home);
        assert_eq!(run(&mut a, 0, 61_000, 12), [(60, ViewId::Calendar)]);
    }

    #[test]
    fn rotation_pauses_after_touch_and_outside_its_hours() {
        let mut a = rotating(ViewId::ALL.to_vec());
        run(&mut a, 0, 70_000, 12);
        // A tap pauses rotation for a minute.
        a.tick(&env_at(70_000, 12), Some(RawTouch { x: 10, y: 10 }));
        let seen = run(&mut a, 70_030, 135_000, 12);
        assert_eq!(seen.first().map(|s| s.0), Some(130));

        a.settings.rotate_from = "08:00".into();
        a.settings.rotate_until = "18:00".into();
        assert!(run(&mut a, 135_000, 300_000, 19).is_empty());
        assert!(!run(&mut a, 300_000, 320_000, 9).is_empty());

        a.settings.rotate = false;
        assert!(run(&mut a, 320_000, 400_000, 12).is_empty());
    }

    #[test]
    fn swipes_move_between_views() {
        let mut a = rotating(ViewId::ALL.to_vec());
        a.settings.rotate = false;
        let swipe = |a: &mut App, t: u64, from: (i32, i32), to: (i32, i32)| {
            for i in 0..=5u64 {
                let x = from.0 + (to.0 - from.0) * i as i32 / 5;
                let y = from.1 + (to.1 - from.1) * i as i32 / 5;
                let raw = RawTouch {
                    x: x as u16,
                    y: y as u16,
                };
                a.tick(&env_at(t + i * 30, 12), Some(raw));
            }
            a.tick(&env_at(t + 200, 12), None);
            a.tick(&env_at(t + 230, 12), None);
        };
        swipe(&mut a, 0, (400, 160), (100, 160));
        assert_eq!(a.view(), ViewId::Forecast);
        swipe(&mut a, 1_000, (240, 250), (240, 60));
        assert_eq!(a.view(), ViewId::Forecast, "vertical swipes do nothing");
        swipe(&mut a, 2_000, (100, 160), (400, 160));
        swipe(&mut a, 3_000, (100, 160), (400, 160));
        assert_eq!(a.view(), ViewId::Calendar, "wraps around");
    }

    #[test]
    fn a_calibration_for_another_layout_is_redone() {
        let mut old = Calibration::identity();
        old.screen = (320, 480);
        let a = App::new(Settings::default(), Some(old));
        assert!(a.calibrate.is_some());
        let a = App::new(Settings::default(), Some(Calibration::identity()));
        assert!(a.calibrate.is_none());
    }
}
