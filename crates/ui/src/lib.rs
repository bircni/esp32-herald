//! Hardware-independent part of the deskbuddy: state, rendering, gestures,
//! and the HTTP API. The firmware and the desktop simulator are thin shells
//! around this crate.

pub mod api;
pub mod app;
pub mod command;
pub mod digits;
pub mod feed;
pub mod gesture;
pub mod gfx;
pub mod model;
pub mod mqtt;
pub mod theme;
pub mod views;
pub mod weather;

pub use app::App;
pub use feed::Item;
pub use model::{
    Beep, Calibration, Effect, Env, ItemColor, Led, LocalTime, NetState, RawTouch, Settings,
    ViewId, SCREEN_H, SCREEN_W,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
