//! Persistent storage in the NVS flash partition.

use anyhow::Result;
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use serde::{Serialize, de::DeserializeOwned};

const NAMESPACE: &str = "herald";
const KEY_SETTINGS: &str = "settings";
const KEY_CALIBRATION: &str = "calibration";
const KEY_WIFI: &str = "wifi";

#[derive(Serialize, serde::Deserialize, Clone)]
pub struct WifiCredentials {
    pub ssid: String,
    pub password: String,
}

pub struct Storage {
    nvs: EspNvs<NvsDefault>,
}

impl Storage {
    pub fn new(partition: EspDefaultNvsPartition) -> Result<Self> {
        Ok(Self {
            nvs: EspNvs::new(partition, NAMESPACE, true)?,
        })
    }

    fn load<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let mut buf = [0u8; 1024];
        let text = match self.nvs.get_str(key, &mut buf) {
            Ok(text) => text?,
            Err(e) => {
                log::warn!("reading {key} failed: {e}");
                return None;
            }
        };
        match serde_json::from_str(text) {
            Ok(value) => Some(value),
            Err(e) => {
                log::warn!("stored {key} is not valid, ignoring it: {e}");
                None
            }
        }
    }

    fn save<T: Serialize>(&self, key: &str, value: &T) {
        let result = serde_json::to_string(value)
            .map_err(anyhow::Error::from)
            .and_then(|text| Ok(self.nvs.set_str(key, &text)?));
        if let Err(e) = result {
            log::error!("saving {key} failed: {e}");
        }
    }

    pub fn settings(&self) -> Option<ui::Settings> {
        self.load::<ui::Settings>(KEY_SETTINGS)
            .filter(|s| s.validate().is_ok())
    }

    pub fn save_settings(&self, settings: &ui::Settings) {
        self.save(KEY_SETTINGS, settings);
    }

    pub fn calibration(&self) -> Option<ui::Calibration> {
        self.load(KEY_CALIBRATION)
    }

    pub fn save_calibration(&self, calibration: &ui::Calibration) {
        self.save(KEY_CALIBRATION, calibration);
    }

    pub fn wifi(&self) -> Option<WifiCredentials> {
        self.load(KEY_WIFI)
    }

    pub fn save_wifi(&self, credentials: &WifiCredentials) {
        self.save(KEY_WIFI, credentials);
    }

    pub fn forget_wifi(&self) {
        if let Err(e) = self.nvs.remove(KEY_WIFI) {
            log::error!("erasing WiFi credentials failed: {e}");
        }
    }
}
