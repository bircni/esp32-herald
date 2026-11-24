//! Getting the app onto a display, two ways:
//!
//! - [`RegionPresenter`]: repaints only the regions whose content changed,
//!   band by band through a small buffer. For colour TFTs without PSRAM,
//!   where a whole frame does not fit in RAM.
//! - [`FramePresenter`]: renders the whole picture as a 1-bit frame and
//!   says how to refresh it. For e-paper; the frame also feeds the web
//!   app's mirror (`/api/frame.bmp`).

mod frame;
pub mod panel;
mod regions;

pub use frame::{FULL_EVERY, Frame, FramePresenter, Refresh};
pub use regions::RegionPresenter;
