//! State shared between the UI loop, the network thread and HTTP handlers.

use std::{
    ffi::CString,
    sync::{Mutex, MutexGuard},
    time::Instant,
};

use esp_idf_svc::sys;
use ui::{App, Beep, Effect, Env, LocalTime, NetState};

use crate::storage::{Storage, WifiCredentials};

/// Delay before a restart, so the HTTP response still gets out.
const RESTART_DELAY_MS: u64 = 1_500;

pub struct Shared {
    app: Mutex<App>,
    net: Mutex<NetState>,
    mqtt: Mutex<Option<bool>>,
    rssi: Mutex<Option<i8>>,
    /// The running firmware slot, e.g. `ota_0`.
    firmware_slot: Option<String>,
    /// Custom screens to publish retained on MQTT: name and payload
    /// (`None` clears). Drained by the MQTT link.
    outbox: Mutex<Vec<(String, Option<String>)>>,
    storage: Mutex<Storage>,
    beeps: Mutex<Vec<Beep>>,
    /// Monotonic time at which to restart, if one is pending.
    restart_at_ms: Mutex<Option<u64>>,
    start: Instant,
}

/// A poisoned lock means another thread panicked; on the device a panic
/// reboots anyway, so the data is used as it is.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Shared {
    pub fn new(app: App, storage: Storage, firmware_slot: Option<String>) -> Self {
        Self {
            app: Mutex::new(app),
            net: Mutex::new(NetState::Offline),
            mqtt: Mutex::new(None),
            rssi: Mutex::new(None),
            firmware_slot,
            outbox: Mutex::new(Vec::new()),
            storage: Mutex::new(storage),
            beeps: Mutex::new(Vec::new()),
            restart_at_ms: Mutex::new(None),
            start: Instant::now(),
        }
    }

    pub fn app(&self) -> MutexGuard<'_, App> {
        lock(&self.app)
    }

    pub fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    pub fn env(&self) -> Env {
        let (unix_s, local) = wall_clock().unzip();
        Env {
            now_ms: self.now_ms(),
            local,
            net: lock(&self.net).clone(),
            unix_s,
            mqtt: *lock(&self.mqtt),
            rssi: *lock(&self.rssi),
            heap_free: Some(free_heap()),
            firmware_slot: self.firmware_slot.clone(),
        }
    }

    pub fn set_rssi(&self, rssi: Option<i8>) {
        *lock(&self.rssi) = rssi;
    }

    /// Screens waiting to be published, and forgets them.
    pub fn take_outbox(&self) -> Vec<(String, Option<String>)> {
        std::mem::take(&mut *lock(&self.outbox))
    }

    /// `None` while MQTT is off, else whether the broker is connected.
    pub fn set_mqtt(&self, state: Option<bool>) {
        *lock(&self.mqtt) = state;
    }

    pub fn set_net(&self, net: NetState) {
        *lock(&self.net) = net;
    }

    pub fn wifi_credentials(&self) -> Option<WifiCredentials> {
        lock(&self.storage).wifi()
    }

    pub fn take_beeps(&self) -> Vec<Beep> {
        std::mem::take(&mut *lock(&self.beeps))
    }

    pub fn restart_due(&self) -> bool {
        lock(&self.restart_at_ms).is_some_and(|at| self.now_ms() >= at)
    }

    pub fn schedule_restart(&self) {
        *lock(&self.restart_at_ms) = Some(self.now_ms() + RESTART_DELAY_MS);
    }

    /// Carries out what the app asked for. Call with the app still locked,
    /// after every tick and every API request.
    pub fn apply_effects(&self, app: &mut App) {
        for effect in app.take_effects() {
            match effect {
                Effect::SaveSettings => lock(&self.storage).save_settings(app.settings()),
                Effect::SaveCalibration(c) => lock(&self.storage).save_calibration(&c),
                Effect::Beep(beep) => lock(&self.beeps).push(beep),
                // WiFi changes take effect through a restart: the network
                // thread reads the credentials once, at boot.
                Effect::StartWifiSetup => {
                    log::info!("WiFi credentials erased, restarting into setup mode");
                    lock(&self.storage).forget_wifi();
                    self.schedule_restart();
                }
                Effect::SetWifi { ssid, password } => {
                    log::info!("WiFi credentials for {ssid:?} saved, restarting");
                    lock(&self.storage).save_wifi(&WifiCredentials { ssid, password });
                    self.schedule_restart();
                }
                Effect::PublishScreen { name, payload } => {
                    lock(&self.outbox).push((name, payload));
                }
                Effect::Restart => {
                    log::info!("restart requested");
                    self.schedule_restart();
                }
            }
        }
    }
}

/// Free heap in bytes.
pub fn free_heap() -> u32 {
    heap_stats().0
}

/// Free heap now and at its lowest point since boot, in bytes.
#[expect(unsafe_code, reason = "ESP-IDF heap statistics are C functions")]
pub fn heap_stats() -> (u32, u32) {
    // SAFETY: plain getters without preconditions.
    unsafe {
        (
            sys::esp_get_free_heap_size(),
            sys::esp_get_minimum_free_heap_size(),
        )
    }
}

/// Applies a POSIX TZ string to the C library's local time conversion.
#[expect(
    unsafe_code,
    reason = "the C library's time zone is set through setenv/tzset"
)]
pub fn set_timezone(tz: &str) {
    let Ok(value) = CString::new(tz) else {
        return;
    };
    // SAFETY: both strings are valid, NUL-terminated and outlive the calls;
    // setenv copies them.
    unsafe {
        sys::setenv(c"TZ".as_ptr(), value.as_ptr(), 1);
        sys::tzset();
    }
}

/// Unix time and local wall-clock time, or `None` while the clock has not
/// been set by SNTP (the ESP32 starts counting from 1970).
#[expect(unsafe_code, reason = "the wall clock comes from the C library")]
fn wall_clock() -> Option<(i64, LocalTime)> {
    // SAFETY: `time` and `localtime_r` only write to the values passed in,
    // and an all-zero `tm` is a valid value for a plain C struct.
    let (now, tm) = unsafe {
        let mut now: sys::time_t = 0;
        sys::time(&raw mut now);
        let mut tm: sys::tm = core::mem::zeroed();
        sys::localtime_r(&raw const now, &raw mut tm);
        (now, tm)
    };
    if tm.tm_year < 2024 - 1900 {
        return None;
    }
    let local = LocalTime {
        year: (tm.tm_year + 1900) as u16,
        month: (tm.tm_mon + 1) as u8,
        day: tm.tm_mday as u8,
        // C counts from Sunday, the UI from Monday.
        weekday: ((tm.tm_wday + 6) % 7) as u8,
        hour: tm.tm_hour as u8,
        minute: tm.tm_min as u8,
        second: tm.tm_sec.min(59) as u8,
    };
    Some((now, local))
}
