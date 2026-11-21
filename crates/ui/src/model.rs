//! Plain data types shared by the UI, the HTTP API and the platforms.

use embedded_graphics::{pixelcolor::Rgb565, prelude::*};
use serde::{Deserialize, Serialize};

pub const SCREEN_W: i32 = 480;
pub const SCREEN_H: i32 = 320;

pub const fn screen() -> embedded_graphics::primitives::Rectangle {
    embedded_graphics::primitives::Rectangle::new(
        Point::new(0, 0),
        Size::new(SCREEN_W as u32, SCREEN_H as u32),
    )
}

/// The swipeable views, in navigation order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewId {
    Home,
    Forecast,
    Calendar,
}

impl ViewId {
    pub const ALL: [ViewId; 3] = [ViewId::Home, ViewId::Forecast, ViewId::Calendar];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|v| *v == self).unwrap_or(0)
    }

    pub fn name(self) -> &'static str {
        match self {
            ViewId::Home => "home",
            ViewId::Forecast => "forecast",
            ViewId::Calendar => "calendar",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.name() == name)
    }
}

/// Colour of the dot in front of a feed item. Also its urgency: items are
/// listed in this order.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "lowercase")]
pub enum ItemColor {
    Red,
    Yellow,
    #[default]
    Blue,
    Green,
    Grey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeKind {
    #[default]
    Dark,
    Light,
}

/// Colour of the on-board RGB LED.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Led {
    #[default]
    Off,
    Red,
    Green,
    Blue,
    Yellow,
    Cyan,
    Magenta,
    White,
}

impl Led {
    /// (red, green, blue) channels that should be lit.
    pub fn channels(self) -> (bool, bool, bool) {
        match self {
            Led::Off => (false, false, false),
            Led::Red => (true, false, false),
            Led::Green => (false, true, false),
            Led::Blue => (false, false, true),
            Led::Yellow => (true, true, false),
            Led::Cyan => (false, true, true),
            Led::Magenta => (true, false, true),
            Led::White => (true, true, true),
        }
    }
}

/// User settings. Persisted as JSON and exposed through `/api/settings`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// POSIX TZ string, e.g. `CET-1CEST,M3.5.0,M10.5.0/3`.
    pub timezone: String,
    /// Display name of the weather location. Empty disables weather.
    pub location: String,
    // f64 rather than f32, see `de_f32`.
    pub latitude: f64,
    pub longitude: f64,
    pub theme: ThemeKind,
    /// Accent colour as `#RRGGBB`.
    pub accent_color: String,
    /// Backlight brightness in percent (5..=100).
    pub brightness: u8,
    /// Seconds without touch until the display sleeps. 0 = never.
    pub sleep_secs: u32,
    /// Optional shared secret required in the `X-Buddy-Token` header.
    pub token: String,
    /// Step through `rotate_views` on its own.
    pub rotate: bool,
    /// Seconds each view is shown while rotating (5..=3600).
    pub rotate_secs: u32,
    /// Views to rotate through, in this order.
    pub rotate_views: Vec<ViewId>,
    /// Local time window (`HH:MM`) in which rotation runs. Equal values
    /// mean all day; `from` after `until` wraps past midnight.
    pub rotate_from: String,
    pub rotate_until: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            timezone: "CET-1CEST,M3.5.0,M10.5.0/3".into(),
            location: String::new(),
            latitude: 0.0,
            longitude: 0.0,
            theme: ThemeKind::Dark,
            accent_color: "#35D0FF".into(),
            brightness: 80,
            sleep_secs: 300,
            token: String::new(),
            rotate: false,
            rotate_secs: 20,
            rotate_views: ViewId::ALL.to_vec(),
            rotate_from: "00:00".into(),
            rotate_until: "00:00".into(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if self.timezone.is_empty() || self.timezone.len() > 64 {
            return Err("timezone must be 1..=64 characters".into());
        }
        if !self
            .timezone
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-,./:<>".contains(c))
        {
            return Err("timezone contains invalid characters".into());
        }
        if self.location.chars().count() > 40 {
            return Err("location must be at most 40 characters".into());
        }
        if !(-90.0..=90.0).contains(&self.latitude) || !(-180.0..=180.0).contains(&self.longitude) {
            return Err("latitude/longitude out of range".into());
        }
        if parse_hex_color(&self.accent_color).is_none() {
            return Err("accent_color must look like #RRGGBB".into());
        }
        if !(5..=100).contains(&self.brightness) {
            return Err("brightness must be 5..=100".into());
        }
        if self.sleep_secs > 86_400 {
            return Err("sleep_secs must be at most 86400".into());
        }
        if self.token.len() > 64 || !self.token.chars().all(|c| c.is_ascii_graphic()) {
            return Err("token must be at most 64 printable ASCII characters".into());
        }
        if !(5..=3600).contains(&self.rotate_secs) {
            return Err("rotate_secs must be 5..=3600".into());
        }
        if self.rotate_views.is_empty() {
            return Err("rotate_views must name at least one view".into());
        }
        for (i, v) in self.rotate_views.iter().enumerate() {
            if self.rotate_views[..i].contains(v) {
                return Err(format!("rotate_views lists {} twice", v.name()));
            }
        }
        if parse_hhmm(&self.rotate_from).is_none() || parse_hhmm(&self.rotate_until).is_none() {
            return Err("rotate_from and rotate_until must look like HH:MM".into());
        }
        Ok(())
    }

    pub fn has_location(&self) -> bool {
        !self.location.is_empty()
    }

    /// Whether the rotation window includes `t`. Without a synchronised
    /// clock only an all-day window counts as open.
    pub fn in_rotate_window(&self, t: Option<&LocalTime>) -> bool {
        let (Some(from), Some(until)) = (
            parse_hhmm(&self.rotate_from),
            parse_hhmm(&self.rotate_until),
        ) else {
            return false;
        };
        if from == until {
            return true;
        }
        let Some(now) = t.map(LocalTime::minute_of_day) else {
            return false;
        };
        if from < until {
            (from..until).contains(&now)
        } else {
            now >= from || now < until
        }
    }
}

/// Parses `HH:MM` into minutes since midnight.
pub fn parse_hhmm(s: &str) -> Option<u16> {
    let (h, m) = s.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u16, u16) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// Parses `#RRGGBB` into an RGB565 colour.
pub fn parse_hex_color(s: &str) -> Option<Rgb565> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    let (r, g, b) = ((v >> 16) as u8, (v >> 8) as u8, v as u8);
    Some(Rgb565::new(r >> 3, g >> 2, b >> 3))
}

/// A raw sample from the resistive touch controller (12-bit ADC values).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct RawTouch {
    pub x: u16,
    pub y: u16,
}

/// Deserialises an `f32` by way of `f64`.
///
/// serde's own `f32` visitor makes the Xtensa LLVM backend of the ESP Rust
/// toolchain abort ("Cannot select: XtensaISD::PCREL_WRAPPER"), so no type
/// in this crate may derive `Deserialize` for a bare `f32` field.
pub(crate) fn de_f32<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    f64::deserialize(d).map(|v| v as f32)
}

/// Maps raw touch ADC values to screen coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    pub swap_xy: bool,
    #[serde(deserialize_with = "de_f32")]
    pub ax: f32,
    #[serde(deserialize_with = "de_f32")]
    pub bx: f32,
    #[serde(deserialize_with = "de_f32")]
    pub ay: f32,
    #[serde(deserialize_with = "de_f32")]
    pub by: f32,
    /// Screen size the calibration was made for. A calibration from another
    /// layout (e.g. portrait) maps to the wrong place and is discarded.
    #[serde(default)]
    pub screen: (i32, i32),
}

impl Calibration {
    /// Raw values already are screen coordinates (used by the simulator).
    pub fn identity() -> Self {
        Self {
            swap_xy: false,
            ax: 1.0,
            bx: 0.0,
            ay: 1.0,
            by: 0.0,
            screen: (SCREEN_W, SCREEN_H),
        }
    }

    pub fn fits_screen(&self) -> bool {
        self.screen == (SCREEN_W, SCREEN_H)
    }

    pub fn map(&self, raw: RawTouch) -> Point {
        let (rx, ry) = if self.swap_xy {
            (raw.y, raw.x)
        } else {
            (raw.x, raw.y)
        };
        let x = (self.ax * f32::from(rx) + self.bx).round() as i32;
        let y = (self.ay * f32::from(ry) + self.by).round() as i32;
        Point::new(x.clamp(0, SCREEN_W - 1), y.clamp(0, SCREEN_H - 1))
    }

    /// Derives a calibration from the four corner targets, given in the order
    /// top-left, top-right, bottom-right, bottom-left.
    pub fn from_corners(targets: &[Point; 4], raw: &[RawTouch; 4]) -> Option<Self> {
        let f = |v: u16| f32::from(v);
        // Does the raw X axis follow the screen X axis, or the screen Y axis?
        let dx_along_x = (f(raw[1].x) - f(raw[0].x)).abs() + (f(raw[2].x) - f(raw[3].x)).abs();
        let dx_along_y = (f(raw[3].x) - f(raw[0].x)).abs() + (f(raw[2].x) - f(raw[1].x)).abs();
        let swap_xy = dx_along_y > dx_along_x;
        let pick = |r: RawTouch| {
            if swap_xy {
                (f(r.y), f(r.x))
            } else {
                (f(r.x), f(r.y))
            }
        };
        let r = [pick(raw[0]), pick(raw[1]), pick(raw[2]), pick(raw[3])];

        let left = (r[0].0 + r[3].0) / 2.0;
        let right = (r[1].0 + r[2].0) / 2.0;
        let top = (r[0].1 + r[1].1) / 2.0;
        let bottom = (r[2].1 + r[3].1) / 2.0;
        // The corners must be clearly apart, otherwise the taps were bogus.
        if (right - left).abs() < 500.0 || (bottom - top).abs() < 500.0 {
            return None;
        }
        let (sx0, sx1) = (targets[0].x as f32, targets[1].x as f32);
        let (sy0, sy1) = (targets[0].y as f32, targets[3].y as f32);
        let ax = (sx1 - sx0) / (right - left);
        let ay = (sy1 - sy0) / (bottom - top);
        Some(Self {
            swap_xy,
            ax,
            bx: sx0 - ax * left,
            ay,
            by: sy0 - ay * top,
            screen: (SCREEN_W, SCREEN_H),
        })
    }
}

/// 0 = Monday .. 6 = Sunday, for any Gregorian date.
pub fn weekday_of(year: u16, month: u8, day: u8) -> u8 {
    // Sakamoto's method, which counts from Sunday.
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let (mut y, m, d) = (
        i32::from(year),
        usize::from(month.clamp(1, 12)),
        i32::from(day),
    );
    if m < 3 {
        y -= 1;
    }
    let sunday_based = (y + y / 4 - y / 100 + y / 400 + T[m - 1] + d).rem_euclid(7);
    ((sunday_based + 6) % 7) as u8
}

/// `Mon` .. `Sun`, for 0 = Monday.
pub fn weekday_abbr(weekday: u8) -> &'static str {
    ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][usize::from(weekday) % 7]
}

pub fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => 31,
    }
}

/// Wall-clock time in the configured time zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    /// 1..=12
    pub month: u8,
    /// 1..=31
    pub day: u8,
    /// 0 = Monday .. 6 = Sunday
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl LocalTime {
    pub fn weekday_name(&self) -> &'static str {
        const NAMES: [&str; 7] = [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ];
        NAMES[usize::from(self.weekday) % 7]
    }

    pub fn month_name(&self) -> &'static str {
        const NAMES: [&str; 12] = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        NAMES[usize::from(self.month.clamp(1, 12)) - 1]
    }

    /// e.g. `19 November 2026`
    pub fn date_string(&self) -> String {
        format!("{} {} {}", self.day, self.month_name(), self.year)
    }

    pub fn minute_of_day(&self) -> u16 {
        u16::from(self.hour) * 60 + u16::from(self.minute)
    }

    /// 1 for 1 January.
    pub fn day_of_year(&self) -> u16 {
        (1..self.month)
            .map(|m| u16::from(days_in_month(self.year, m)))
            .sum::<u16>()
            + u16::from(self.day)
    }

    /// ISO 8601 week number.
    pub fn iso_week(&self) -> u8 {
        let thursday = i32::from(self.day_of_year()) - i32::from(self.weekday) + 3;
        let year_days = |y: u16| if days_in_month(y, 2) == 29 { 366 } else { 365 };
        if thursday < 1 {
            // Belongs to the last week of the previous year.
            let prev = self.year - 1;
            let dec31 = LocalTime {
                year: prev,
                month: 12,
                day: 31,
                weekday: weekday_of(prev, 12, 31),
                ..*self
            };
            return dec31.iso_week();
        }
        if thursday > year_days(self.year) {
            return 1;
        }
        ((thursday - 1) / 7 + 1) as u8
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum NetState {
    #[default]
    Offline,
    Connecting {
        ssid: String,
    },
    Connected {
        ip: String,
    },
    /// The device runs its own access point and waits for WiFi credentials.
    Setup {
        ssid: String,
        password: String,
        /// Address of the device on its own network (ESP-IDF 5.5 uses
        /// 192.168.71.1, older versions 192.168.4.1).
        ip: String,
    },
}

/// Everything the platform tells the app on each tick.
#[derive(Clone, Debug, Default)]
pub struct Env {
    /// Monotonic milliseconds since boot.
    pub now_ms: u64,
    /// `None` until the clock has been synchronised.
    pub local: Option<LocalTime>,
    pub net: NetState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Beep {
    /// Two short beeps, asked for by a feed item.
    Notice,
}

/// Side effects the platform has to carry out after a tick or an API call.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    SaveSettings,
    SaveCalibration(Calibration),
    Beep(Beep),
    /// Forget the WiFi credentials and open the setup access point.
    StartWifiSetup,
    /// Store new WiFi credentials and reconnect.
    SetWifi {
        ssid: String,
        password: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors() {
        assert_eq!(parse_hex_color("#FFFFFF"), Some(Rgb565::WHITE));
        assert_eq!(parse_hex_color("#000000"), Some(Rgb565::BLACK));
        assert_eq!(parse_hex_color("#ff0000"), Some(Rgb565::RED));
        assert_eq!(parse_hex_color("FFFFFF"), None);
        assert_eq!(parse_hex_color("#FFF"), None);
        assert_eq!(parse_hex_color("#GGGGGG"), None);
        assert_eq!(parse_hex_color("#ääa"), None);
    }

    #[test]
    fn default_settings_are_valid() {
        assert_eq!(Settings::default().validate(), Ok(()));
    }

    #[test]
    fn settings_validation_rejects_bad_values() {
        let ok = Settings::default();
        let mut s = ok.clone();
        s.brightness = 0;
        assert!(s.validate().is_err());
        let mut s = ok.clone();
        s.timezone = "CET; rm -rf".into();
        assert!(s.validate().is_err());
        let mut s = ok.clone();
        s.latitude = 91.0;
        assert!(s.validate().is_err());
        let mut s = ok.clone();
        s.accent_color = "blue".into();
        assert!(s.validate().is_err());
        let mut s = ok;
        s.token = "has space".into();
        assert!(s.validate().is_err());
    }

    const TARGETS: [Point; 4] = [
        Point::new(30, 30),
        Point::new(450, 30),
        Point::new(450, 290),
        Point::new(30, 290),
    ];

    #[test]
    fn calibration_plain_axes() {
        // raw = 10 * screen + 200
        let raw = TARGETS.map(|p| RawTouch {
            x: (p.x * 10 + 200) as u16,
            y: (p.y * 8 + 100) as u16,
        });
        let c = Calibration::from_corners(&TARGETS, &raw).unwrap();
        assert!(!c.swap_xy);
        assert_eq!(c.map(RawTouch { x: 2600, y: 1380 }), Point::new(240, 160));
    }

    #[test]
    fn calibration_swapped_and_inverted_axes() {
        // raw.x follows screen Y, raw.y follows inverted screen X.
        let raw = TARGETS.map(|p| RawTouch {
            x: (p.y * 10 + 300) as u16,
            y: (4000 - p.x * 8) as u16,
        });
        let c = Calibration::from_corners(&TARGETS, &raw).unwrap();
        assert!(c.swap_xy);
        let p = c.map(RawTouch {
            x: (160 * 10 + 300) as u16,
            y: (4000 - 240 * 8) as u16,
        });
        assert_eq!(p, Point::new(240, 160));
    }

    #[test]
    fn calibration_rejects_degenerate_taps() {
        let raw = [RawTouch { x: 2000, y: 2000 }; 4];
        assert_eq!(Calibration::from_corners(&TARGETS, &raw), None);
    }

    #[test]
    fn calibration_survives_storage() {
        let c = Calibration {
            swap_xy: true,
            ax: 0.0912,
            bx: -22.5,
            ay: -0.1431,
            by: 512.0,
            screen: (SCREEN_W, SCREEN_H),
        };
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Calibration>(&json).unwrap(), c);
        // Whole numbers are written without a fraction by some tools.
        let c: Calibration = serde_json::from_str(
            r#"{"swap_xy":false,"ax":1,"bx":0,"ay":1,"by":0,"screen":[480,320]}"#,
        )
        .unwrap();
        assert_eq!(c, Calibration::identity());
    }

    #[test]
    fn calibration_from_the_portrait_layout_does_not_fit() {
        let old: Calibration =
            serde_json::from_str(r#"{"swap_xy":true,"ax":0.1,"bx":0,"ay":0.1,"by":0}"#).unwrap();
        assert!(!old.fits_screen());
        assert!(Calibration::identity().fits_screen());
    }

    #[test]
    fn calibration_clamps_to_screen() {
        let c = Calibration::identity();
        assert_eq!(c.map(RawTouch { x: 4000, y: 4000 }), Point::new(479, 319));
    }

    #[test]
    fn date_formatting() {
        let t = LocalTime {
            year: 2026,
            month: 11,
            day: 19,
            weekday: 3,
            hour: 18,
            minute: 5,
            second: 0,
        };
        assert_eq!(t.weekday_name(), "Thursday");
        assert_eq!(t.date_string(), "19 November 2026");
        assert_eq!(t.minute_of_day(), 18 * 60 + 5);
    }

    fn date(year: u16, month: u8, day: u8) -> LocalTime {
        LocalTime {
            year,
            month,
            day,
            weekday: weekday_of(year, month, day),
            hour: 12,
            minute: 0,
            second: 0,
        }
    }

    #[test]
    fn calendar_arithmetic() {
        assert_eq!(weekday_of(2026, 11, 19), 3, "Thursday");
        assert_eq!(weekday_of(2024, 2, 29), 3, "Thursday");
        assert_eq!(weekday_of(2000, 1, 1), 5, "Saturday");
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2100, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2026, 11), 30);
        assert_eq!(date(2026, 1, 1).day_of_year(), 1);
        assert_eq!(date(2024, 12, 31).day_of_year(), 366);
        // ISO weeks, including the edges of the year.
        assert_eq!(date(2026, 11, 19).iso_week(), 47);
        assert_eq!(date(2021, 1, 3).iso_week(), 53, "belongs to 2020");
        assert_eq!(date(2024, 12, 30).iso_week(), 1, "belongs to 2025");
        assert_eq!(date(2026, 1, 1).iso_week(), 1);
        assert_eq!(date(2027, 1, 1).iso_week(), 53, "belongs to 2026");
    }

    #[test]
    fn hhmm() {
        assert_eq!(parse_hhmm("00:00"), Some(0));
        assert_eq!(parse_hhmm("07:30"), Some(450));
        assert_eq!(parse_hhmm("23:59"), Some(1439));
        for bad in ["24:00", "7:30", "07:60", "0730", "", "ab:cd", "07:3"] {
            assert_eq!(parse_hhmm(bad), None, "{bad}");
        }
    }

    #[test]
    fn rotate_window() {
        let at = |h: u8, m: u8| LocalTime {
            hour: h,
            minute: m,
            ..date(2026, 11, 19)
        };
        let mut s = Settings::default();
        assert!(s.in_rotate_window(None), "all day, even without a clock");
        assert!(s.in_rotate_window(Some(&at(3, 0))));

        s.rotate_from = "08:00".into();
        s.rotate_until = "18:30".into();
        assert!(!s.in_rotate_window(None));
        assert!(!s.in_rotate_window(Some(&at(7, 59))));
        assert!(s.in_rotate_window(Some(&at(8, 0))));
        assert!(s.in_rotate_window(Some(&at(18, 29))));
        assert!(!s.in_rotate_window(Some(&at(18, 30))));

        // Across midnight.
        s.rotate_from = "22:00".into();
        s.rotate_until = "06:00".into();
        assert!(s.in_rotate_window(Some(&at(23, 0))));
        assert!(s.in_rotate_window(Some(&at(5, 59))));
        assert!(!s.in_rotate_window(Some(&at(12, 0))));
    }

    #[test]
    fn rotation_settings_are_validated() {
        let ok = Settings::default();
        let mut s = ok.clone();
        s.rotate_secs = 2;
        assert!(s.validate().is_err());
        let mut s = ok.clone();
        s.rotate_views.clear();
        assert!(s.validate().is_err());
        let mut s = ok.clone();
        s.rotate_views = vec![ViewId::Home, ViewId::Home];
        assert!(s.validate().is_err());
        let mut s = ok;
        s.rotate_until = "25:00".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn settings_from_older_firmware_load_with_defaults() {
        let s: Settings = serde_json::from_str(r#"{"brightness":40}"#).unwrap();
        assert_eq!(s.brightness, 40);
        assert_eq!(s.rotate_views, ViewId::ALL.to_vec());
        assert_eq!(s.validate(), Ok(()));
    }
}
