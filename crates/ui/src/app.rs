//! The application: owns all state, reacts to touch and API calls, and
//! repaints only the screen regions whose content changed.

use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};

use crate::{
    feed::{Feed, Item},
    gesture::{Dir, Gesture, Recognizer},
    gfx::{self, Canvas},
    model::{
        ALL_SCREENS, Beep, Calibration, Effect, Env, ItemColor, Led, NetState, RawTouch, Settings,
        ViewId, screen,
    },
    screen::{Screen, Screens},
    theme::Theme,
    views::{self, Region},
    weather::{self, Weather},
};

/// After a touch, rotation waits this long before it moves on.
pub const ROTATE_TOUCH_PAUSE_MS: u64 = 60_000;

/// What currently fills the display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Showing {
    Calibrate,
    WifiSetup,
    Menu,
    View(ViewId),
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "independent UI flags, not a state machine"
)]
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
    /// When the weather was last set, to drop it once stale.
    weather_ms: u64,
    pub(crate) screens: Screens,
    /// Night mode applies right now (updated every tick).
    night_now: bool,
    /// Night mode applies and nothing has lit the screen (every tick).
    night_dim: bool,
    /// At night, a touch or an alert lights the screen until then.
    night_wake_until_ms: u64,
    gesture: Recognizer,
    last_activity_ms: u64,
    /// When the view last changed, for rotation.
    last_switch_ms: u64,
    pub(crate) asleep: bool,
    /// The touch that woke the display must not trigger anything else.
    swallow_touch: bool,
    effects: Vec<Effect>,
    /// On e-paper boards: the picture the panel shows and a counter that
    /// changes with it, for the web app.
    pub(crate) shown_frame: Option<(u64, crate::epaper::Frame)>,
    buf: Vec<Rgb565>,
    cache: Vec<u64>,
    cache_screen: Option<(Showing, i64)>,
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
            weather_ms: 0,
            screens: Screens::default(),
            night_now: false,
            night_dim: false,
            night_wake_until_ms: 0,
            gesture: Recognizer::new(),
            last_activity_ms: 0,
            last_switch_ms: 0,
            asleep: false,
            swallow_touch: false,
            effects: Vec::new(),
            shown_frame: None,
            buf: vec![Rgb565::BLACK; gfx::BAND_PIXELS],
            cache: Vec::new(),
            cache_screen: None,
        }
    }

    // ------------------------------------------------------------ accessors

    #[must_use]
    pub const fn settings(&self) -> &Settings {
        &self.settings
    }

    #[must_use]
    pub const fn view(&self) -> ViewId {
        self.view
    }

    #[must_use]
    pub fn feed(&self) -> &[Item] {
        self.feed.items()
    }

    #[must_use]
    pub const fn weather(&self) -> Option<&Weather> {
        self.weather.as_ref()
    }

    #[must_use]
    pub const fn is_asleep(&self) -> bool {
        self.asleep
    }

    /// Side effects for the platform to carry out. Call after every tick and
    /// every API request.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// Backlight level in percent.
    #[must_use]
    pub const fn backlight(&self) -> u8 {
        if self.asleep {
            0
        } else if self.night_dimmed() {
            self.settings.night_brightness
        } else {
            self.settings.brightness
        }
    }

    /// Night mode applies and nothing has lit the screen.
    #[must_use]
    pub const fn night_dimmed(&self) -> bool {
        self.night_dim
    }

    /// Whether night mode applies right now.
    #[must_use]
    pub const fn is_night(&self) -> bool {
        self.night_now
    }

    #[must_use]
    pub const fn screens(&self) -> &Screens {
        &self.screens
    }

    /// Views in swipe order: the built-in ones, then custom screens by name.
    #[must_use]
    pub fn nav_order(&self) -> Vec<ViewId> {
        ViewId::BUILT_IN
            .into_iter()
            .chain(self.screens.ordered().into_iter().map(ViewId::Screen))
            .collect()
    }

    #[must_use]
    pub fn view_name(&self, view: ViewId) -> String {
        match (view.built_in_name(), view) {
            (Some(name), _) => name.to_owned(),
            (None, ViewId::Screen(slot)) => self
                .screens
                .get(slot)
                .map_or_else(String::new, |e| e.name.clone()),
            (None, ViewId::Home | ViewId::Forecast | ViewId::Calendar) => String::new(),
        }
    }

    /// A built-in view or a custom screen by name.
    #[must_use]
    pub fn view_by_name(&self, name: &str) -> Option<ViewId> {
        ViewId::built_in(name).or_else(|| self.screens.slot(name).map(ViewId::Screen))
    }

    /// A red or yellow item at the top of the feed lights the LED in its
    /// colour.
    #[must_use]
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
        self.light_at_night(now_ms);
    }

    /// Lights a night-dimmed screen for `sleep_secs` (at least 30 s).
    fn light_at_night(&mut self, now_ms: u64) {
        let secs = u64::from(self.settings.sleep_secs.max(30));
        self.night_wake_until_ms = now_ms + secs * 1000;
        self.night_dim = false;
    }

    /// Sends the display to sleep now, e.g. from the web page.
    pub const fn sleep(&mut self) {
        self.asleep = true;
        self.menu_open = false;
    }

    /// Adds or replaces a custom screen.
    ///
    /// # Errors
    /// When the screen is invalid or all slots are taken.
    pub fn put_screen(&mut self, name: &str, screen: Screen, now_ms: u64) -> Result<(), String> {
        self.screens.put(name, screen, now_ms).map(|_| ())
    }

    pub fn remove_screen(&mut self, name: &str) -> bool {
        let removed = self.screens.remove(name);
        self.leave_missing_screen();
        removed
    }

    /// Goes home if the shown custom screen no longer exists.
    fn leave_missing_screen(&mut self) {
        if let ViewId::Screen(slot) = self.view
            && self.screens.get(slot).is_none()
        {
            self.view = ViewId::Home;
        }
    }

    /// A gesture made on the web preview, handled as if it was made on the
    /// screen: it wakes a sleeping display and otherwise does what a touch
    /// would.
    pub fn remote_gesture(&mut self, gesture: Gesture, env: &Env) {
        let was_asleep = self.asleep;
        self.wake(env.now_ms);
        if !was_asleep {
            self.on_gesture(gesture, env);
        }
    }

    pub const fn set_view(&mut self, view: ViewId, now_ms: u64) {
        self.view = view;
        self.menu_open = false;
        self.last_switch_ms = now_ms;
    }

    /// Replaces the settings after validating them.
    ///
    /// # Errors
    /// When the new settings are invalid; the old ones stay.
    pub fn update_settings(&mut self, new: Settings) -> Result<(), String> {
        new.validate()?;
        if new != self.settings {
            self.settings = new;
            self.effects.push(Effect::SaveSettings);
        }
        Ok(())
    }

    /// Adds or replaces a feed item; see [`Feed::upsert`].
    ///
    /// # Errors
    /// When the feed is full.
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

    pub fn clear_weather(&mut self) {
        self.weather = None;
    }

    /// Replaces the weather, e.g. from an MQTT message.
    pub fn set_weather(&mut self, mut w: Weather, env: &Env) {
        w.updated = env.local.map(|t| (t.hour, t.minute));
        w.date_hours(env.local.as_ref());
        self.weather = Some(w);
        self.weather_ms = env.now_ms;
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
            // A dark screen only wakes; the touch does nothing else.
            let dark = self.asleep || (self.night_dim && self.settings.night_brightness == 0);
            if dark && !self.swallow_touch {
                self.swallow_touch = true;
            }
            self.asleep = false;
            self.last_activity_ms = now;
            if self.night_now {
                self.light_at_night(now);
            }
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
        self.screens.expire(now);
        self.leave_missing_screen();
        if now.saturating_sub(self.weather_ms) >= weather::STALE_MS {
            self.weather = None;
        }
        self.night_now = self.settings.is_night(env.local.as_ref());
        self.night_dim = self.night_now && now >= self.night_wake_until_ms;
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
        let showing_view = matches!(self.showing(env), Showing::View(_));
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
        let order = self.rotation_order();
        let Some(&first) = order.first() else {
            return;
        };
        let next = order
            .iter()
            .position(|v| *v == self.view)
            .map_or(first, |i| order[(i + 1) % order.len()]);
        self.view = next;
        self.last_switch_ms = now;
    }

    /// `rotate_views` resolved to existing views, without repeats.
    fn rotation_order(&self) -> Vec<ViewId> {
        let mut order = Vec::new();
        for name in &self.settings.rotate_views {
            let views = if name == ALL_SCREENS {
                self.screens
                    .ordered()
                    .into_iter()
                    .map(ViewId::Screen)
                    .collect()
            } else {
                self.view_by_name(name).into_iter().collect::<Vec<_>>()
            };
            for v in views {
                if !order.contains(&v) {
                    order.push(v);
                }
            }
        }
        order
    }

    pub(crate) const fn showing(&self, env: &Env) -> Showing {
        if self.calibrate.is_some() {
            Showing::Calibrate
        } else if self.menu_open {
            Showing::Menu
        } else if matches!(env.net, NetState::Setup { .. }) && !self.setup_dismissed {
            Showing::WifiSetup
        } else {
            Showing::View(self.view)
        }
    }

    fn on_gesture(&mut self, gesture: Gesture, env: &Env) {
        match self.showing(env) {
            Showing::Calibrate => {}
            Showing::WifiSetup => match gesture {
                Gesture::Tap(_) => self.setup_dismissed = true,
                Gesture::LongPress(_) => self.menu_open = true,
                Gesture::Swipe(_) => {}
            },
            Showing::Menu => {
                if let Gesture::Tap(p) = gesture {
                    views::menu::tap(self, p);
                }
            }
            Showing::View(_) => match gesture {
                Gesture::LongPress(_) => {
                    self.menu_open = true;
                    self.menu_confirm_wifi = false;
                }
                Gesture::Swipe(Dir::Left) => self.step_view(true, env.now_ms),
                Gesture::Swipe(Dir::Right) => self.step_view(false, env.now_ms),
                Gesture::Swipe(_) | Gesture::Tap(_) => {}
            },
        }
    }

    fn step_view(&mut self, forward: bool, now_ms: u64) {
        let order = self.nav_order();
        let i = order.iter().position(|v| *v == self.view).unwrap_or(0);
        let n = order.len();
        let next = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        self.set_view(order[next], now_ms);
    }

    /// A button or touch: wakes the screen and pauses rotation.
    pub(crate) const fn touched(&mut self, now_ms: u64) {
        self.asleep = false;
        self.last_activity_ms = now_ms;
    }

    pub(crate) fn start_calibration(&mut self) {
        self.menu_open = false;
        self.calibrate = Some(views::setup::CalibrateState::default());
    }

    // ---------------------------------------------------------------- draw

    /// Position of the current view in swipe order, and how many there are.
    fn nav_position(&self) -> (usize, usize) {
        let order = self.nav_order();
        let i = order.iter().position(|v| *v == self.view).unwrap_or(0);
        (i, order.len())
    }

    fn regions(&self, showing: Showing, env: &Env) -> Vec<Region> {
        match showing {
            Showing::Calibrate => views::setup::calibrate_regions(self),
            Showing::WifiSetup => views::setup::wifi_regions(env),
            Showing::Menu => views::menu::regions(self, env),
            Showing::View(view) => {
                let mut regions = match view {
                    ViewId::Home => views::home::regions(self, env),
                    ViewId::Forecast => views::forecast::regions(self, env),
                    ViewId::Calendar => views::calendar::regions(self, env),
                    ViewId::Screen(slot) => views::custom::regions(self, slot, env),
                };
                let (i, n) = self.nav_position();
                regions.push(views::nav_region(i, n));
                regions
            }
        }
    }

    fn paint(&self, showing: Showing, c: &mut Canvas<'_>, env: &Env, theme: &Theme) {
        gfx::fill(c, self::screen(), theme.bg);
        match showing {
            Showing::Calibrate => views::setup::paint_calibrate(self, c, theme),
            Showing::WifiSetup => views::setup::paint_wifi(c, env, theme),
            Showing::Menu => views::menu::paint(self, c, env, theme),
            Showing::View(view) => {
                match view {
                    ViewId::Home => views::home::paint(self, c, env, theme),
                    ViewId::Forecast => views::forecast::paint(self, c, env, theme),
                    ViewId::Calendar => views::calendar::paint(self, c, env, theme),
                    ViewId::Screen(slot) => views::custom::paint(self, slot, c, env, theme),
                }
                let (i, n) = self.nav_position();
                views::paint_nav(c, i, n, theme);
            }
        }
    }

    /// Makes the next [`App::draw`] repaint the whole screen, e.g. after the
    /// display was re-initialised.
    pub const fn invalidate(&mut self) {
        self.cache_screen = None;
    }

    /// Repaints whatever changed since the last call. The first call, a
    /// change of screen and a change of theme repaint everything.
    ///
    /// # Errors
    /// When the display fails; the next call repaints what is missing.
    pub fn draw<D>(&mut self, display: &mut D, env: &Env) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        if self.asleep {
            return Ok(());
        }
        let showing = self.showing(env);
        let theme = self.theme();
        let regions = self.regions(showing, env);
        let cache_id = Some((showing, theme.key()));
        let full = self.cache_screen != cache_id || self.cache.len() != regions.len();

        let mut buf = std::mem::take(&mut self.buf);
        let mut paint = |rect: Rectangle| {
            gfx::render(display, &mut buf, rect, |c| {
                self.paint(showing, c, env, &theme);
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

    fn rotating(views: &[&str]) -> App {
        let settings = Settings {
            rotate: true,
            rotate_secs: 10,
            rotate_views: views.iter().map(|v| (*v).to_owned()).collect(),
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
        let mut a = rotating(&["home", "calendar", "forecast"]);
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
        let mut a = rotating(&["calendar", "forecast"]);
        assert_eq!(a.view(), ViewId::Home);
        assert_eq!(run(&mut a, 0, 61_000, 12), [(60, ViewId::Calendar)]);
    }

    #[test]
    fn rotation_pauses_after_touch_and_outside_its_hours() {
        let mut a = rotating(&["home", "forecast", "calendar"]);
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
        let mut a = rotating(&["home", "forecast", "calendar"]);
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
