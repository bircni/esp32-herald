//! Hardware-independent part of Herald: state, rendering, gestures,
//! and the HTTP API. The firmware and the desktop simulator are thin shells
//! around this crate.

pub mod api;
pub mod app;
pub mod color;
pub mod command;
pub mod de;
pub mod dns;
pub mod epaper;
pub mod feed;
pub mod gesture;
pub mod gfx;
pub mod logbuf;
pub mod model;
pub mod mqtt;
pub mod profile;
pub mod screen;
pub mod theme;
pub mod vector;
pub mod views;
pub mod weather;

pub use app::App;
pub use feed::Item;
pub use model::{
    Beep, Calibration, Effect, Env, ItemColor, Led, LocalTime, NetState, RawTouch, Settings, ViewId,
};
pub use profile::Profile;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
