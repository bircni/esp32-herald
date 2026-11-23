//! What every Herald firmware shares: `WiFi` and the setup access point,
//! the web app and HTTP API, MQTT, settings storage, updates over `WiFi`,
//! the log, and the main loop. A board crate provides a [`Board`] (pins, display,
//! input, outputs) and calls [`start`] and [`run`].

mod logger;
mod mqtt;
mod net;
mod run;
mod shared;
mod storage;

pub use run::{Board, Outputs, Start, run, start};
pub use shared::Shared;
