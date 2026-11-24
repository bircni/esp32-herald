//! What the displays say, in English or German (the `language` setting).
//! The web app stays English.

use serde::{Deserialize, Serialize};

use crate::{model::LocalTime, weather::Condition};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    De,
}

/// Fixed texts on the displays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Text {
    AllQuiet,
    NoWeatherYet,
    ComesOverMqtt,
    NoWeather,
    NoForecastYet,
    SendOverMqtt,
    NextHours,
    Today,
    WaitingForTimeSync,
    NoWifiNoTime,
    WaitingForTime,
    TouchCalibration,
    PressAndHold,
    TryAgain,
    WifiSetup,
    JoinNetwork,
    Password,
    OpenInBrowser,
    EnterHomeWifi,
    Band,
    TapToContinue,
    Menu,
    WifiOffline,
    CalibrateTouch,
    ResetWifiConfirm,
    Close,
}

const WEEKDAYS_EN: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const WEEKDAYS_DE: [&str; 7] = [
    "Montag",
    "Dienstag",
    "Mittwoch",
    "Donnerstag",
    "Freitag",
    "Samstag",
    "Sonntag",
];
const MONTHS_EN: [&str; 12] = [
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
const MONTHS_DE: [&str; 12] = [
    "Januar",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];

impl Language {
    #[must_use]
    pub const fn text(self, t: Text) -> &'static str {
        let (en, de) = match t {
            Text::AllQuiet => ("All quiet", "Alles ruhig"),
            Text::NoWeatherYet => ("No weather yet", "Noch kein Wetter"),
            Text::ComesOverMqtt => ("Comes in over MQTT", "Kommt über MQTT"),
            Text::NoWeather => ("No weather", "Kein Wetter"),
            Text::NoForecastYet => ("No forecast yet", "Noch keine Vorhersage"),
            Text::SendOverMqtt => ("Send one over MQTT", "Per MQTT senden"),
            Text::NextHours => ("Next hours", "Nächste Stunden"),
            Text::Today => ("Today", "Heute"),
            Text::WaitingForTimeSync => ("Waiting for time sync", "Warte auf die Uhrzeit"),
            Text::NoWifiNoTime => ("No WiFi, time not set", "Kein WLAN, keine Uhrzeit"),
            Text::WaitingForTime => ("Waiting for the time", "Warte auf die Uhrzeit"),
            Text::TouchCalibration => ("Touch calibration", "Touch kalibrieren"),
            Text::PressAndHold => (
                "Press and hold the centre of the cross",
                "Die Mitte des Kreuzes gedrückt halten",
            ),
            Text::TryAgain => (
                "That did not work, please try again",
                "Hat nicht geklappt, bitte noch einmal",
            ),
            Text::WifiSetup => ("WiFi setup", "WLAN einrichten"),
            Text::JoinNetwork => ("1. Join this WiFi network", "1. Mit diesem WLAN verbinden"),
            Text::Password => ("Password", "Passwort"),
            Text::OpenInBrowser => ("2. Open in your browser", "2. Im Browser öffnen"),
            Text::EnterHomeWifi => ("3. Enter your home WiFi", "3. Eigenes WLAN eintragen"),
            Text::Band => ("(2.4 GHz)", "(2,4 GHz)"),
            Text::TapToContinue => ("Tap to continue without WiFi", "Tippen: ohne WLAN weiter"),
            Text::Menu => ("Menu", "Menü"),
            Text::WifiOffline => ("WiFi: offline", "WLAN: offline"),
            Text::CalibrateTouch => ("Calibrate touch", "Touch kalibrieren"),
            Text::ResetWifiConfirm => (
                "Tap again to reset WiFi",
                "Nochmal tippen: WLAN zurücksetzen",
            ),
            Text::Close => ("Close", "Schließen"),
        };
        match self {
            Self::En => en,
            Self::De => de,
        }
    }

    /// "+3 more"
    #[must_use]
    pub fn more(self, n: usize) -> String {
        match self {
            Self::En => format!("+{n} more"),
            Self::De => format!("+{n} weitere"),
        }
    }

    #[must_use]
    pub fn connecting(self, ssid: &str) -> String {
        match self {
            Self::En => format!("Connecting to {ssid}"),
            Self::De => format!("Verbinde mit {ssid}"),
        }
    }

    #[must_use]
    pub fn setup_network(self, ssid: &str) -> String {
        match self {
            Self::En => format!("Setup network {ssid}"),
            Self::De => format!("Einrichtungs-WLAN {ssid}"),
        }
    }

    #[must_use]
    pub fn brightness(self, percent: u8) -> String {
        match self {
            Self::En => format!("Brightness {percent}%"),
            Self::De => format!("Helligkeit {percent} %"),
        }
    }

    #[must_use]
    pub const fn theme(self, dark: bool) -> &'static str {
        match (self, dark) {
            (Self::En, true) => "Theme: dark",
            (Self::En, false) => "Theme: light",
            (Self::De, true) => "Design: dunkel",
            (Self::De, false) => "Design: hell",
        }
    }

    #[must_use]
    pub const fn rotate(self, on: bool) -> &'static str {
        match (self, on) {
            (Self::En, true) => "Auto-rotate: on",
            (Self::En, false) => "Auto-rotate: off",
            (Self::De, true) => "Wechseln: an",
            (Self::De, false) => "Wechseln: aus",
        }
    }

    /// "Monday", for 0 = Monday.
    #[must_use]
    pub fn weekday(self, weekday: u8) -> &'static str {
        let i = usize::from(weekday) % 7;
        match self {
            Self::En => WEEKDAYS_EN[i],
            Self::De => WEEKDAYS_DE[i],
        }
    }

    /// "Mon" / "Mo".
    #[must_use]
    pub fn weekday_abbr(self, weekday: u8) -> &'static str {
        let name = self.weekday(weekday);
        name.get(..self.abbr_len()).unwrap_or(name)
    }

    /// Two letters, for the calendar's header: "Mo", "Tu" / "Mo", "Di".
    #[must_use]
    pub fn weekday_letters(self, weekday: u8) -> &'static str {
        let name = self.weekday(weekday);
        name.get(..2).unwrap_or(name)
    }

    const fn abbr_len(self) -> usize {
        match self {
            Self::En => 3,
            Self::De => 2,
        }
    }

    /// "January", for 1 = January.
    #[must_use]
    pub fn month(self, month: u8) -> &'static str {
        let i = usize::from(month.clamp(1, 12)) - 1;
        match self {
            Self::En => MONTHS_EN[i],
            Self::De => MONTHS_DE[i],
        }
    }

    /// "Jan", "Mär".
    #[must_use]
    pub fn month_abbr(self, month: u8) -> String {
        self.month(month).chars().take(3).collect()
    }

    /// "Sat 21 November" / "Sa, 21. November".
    #[must_use]
    pub fn date(self, t: &LocalTime) -> String {
        let (day, month) = (self.weekday_abbr(t.weekday), self.month(t.month));
        match self {
            Self::En => format!("{day} {} {month}", t.day),
            Self::De => format!("{day}, {}. {month}", t.day),
        }
    }

    /// "Saturday 21 Nov" / "Samstag, 21. Nov"; with `short` the weekday is
    /// abbreviated too.
    #[must_use]
    pub fn date_with_weekday(self, t: &LocalTime, short: bool) -> String {
        let day = if short {
            self.weekday_abbr(t.weekday)
        } else {
            self.weekday(t.weekday)
        };
        let month = self.month_abbr(t.month);
        match self {
            Self::En => format!("{day} {} {month}", t.day),
            Self::De => format!("{day}, {}. {month}", t.day),
        }
    }

    /// "November 2026"
    #[must_use]
    pub fn month_year(self, t: &LocalTime) -> String {
        format!("{} {}", self.month(t.month), t.year)
    }

    #[must_use]
    pub const fn condition(self, c: Condition) -> &'static str {
        match self {
            Self::En => c.label(),
            Self::De => match c {
                Condition::Clear => "Klar",
                Condition::PartlyCloudy => "Teils bewölkt",
                Condition::Cloudy => "Bewölkt",
                Condition::Fog => "Nebel",
                Condition::Drizzle => "Nieselregen",
                Condition::Rain => "Regen",
                Condition::Snow => "Schnee",
                Condition::Thunder => "Gewitter",
            },
        }
    }

    /// Compact age: `now`, `12m`, `3h 05m`, `2d 4h` / `jetzt`, ..., `2T 4h`.
    #[must_use]
    pub fn age(self, ms: u64) -> String {
        let m = ms / 60_000;
        let (now, day) = match self {
            Self::En => ("now", "d"),
            Self::De => ("jetzt", "T"),
        };
        match m {
            0 => now.into(),
            1..=59 => format!("{m}m"),
            60..=1439 => format!("{}h {:02}m", m / 60, m % 60),
            _ => format!("{}{day} {}h", m / 1440, m % 1440 / 60),
        }
    }

    /// The letter for days in countdowns: `2d 4h` / `2T 4h`.
    #[must_use]
    pub const fn day_letter(self) -> &'static str {
        match self {
            Self::En => "d",
            Self::De => "T",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saturday() -> LocalTime {
        LocalTime {
            year: 2026,
            month: 3,
            day: 21,
            weekday: 5,
            hour: 17,
            minute: 42,
            second: 0,
        }
    }

    #[test]
    fn dates_read_naturally() {
        let t = saturday();
        assert_eq!(Language::En.date(&t), "Sat 21 March");
        assert_eq!(Language::De.date(&t), "Sa, 21. März");
        assert_eq!(
            Language::De.date_with_weekday(&t, false),
            "Samstag, 21. Mär"
        );
        assert_eq!(Language::En.date_with_weekday(&t, true), "Sat 21 Mar");
        assert_eq!(Language::De.weekday_letters(1), "Di");
        assert_eq!(Language::En.weekday_letters(1), "Tu");
    }

    #[test]
    fn ages_and_conditions() {
        assert_eq!(Language::En.age(0), "now");
        assert_eq!(Language::De.age(0), "jetzt");
        assert_eq!(Language::De.age(3 * 86_400_000), "3T 0h");
        assert_eq!(
            Language::De.condition(Condition::PartlyCloudy),
            "Teils bewölkt"
        );
        assert_eq!(Language::En.condition(Condition::Thunder), "Thunderstorm");
        assert_eq!(Language::De.more(7), "+7 weitere");
    }
}
