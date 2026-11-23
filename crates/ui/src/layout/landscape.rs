//! The landscape layout, for 480x320 colour TFTs.
//!
//! Home, forecast, calendar and custom screens are views to swipe between;
//! there are also the menu, touch calibration and the `WiFi` setup screen.
//! The views live in [`crate::views`]; this decides which one fills the
//! screen and which of its regions changed.

use crate::{
    app::{App, Showing},
    gfx::{self, Canvas},
    model::{Env, ViewId},
    theme::Theme,
    views::{self, Region},
};

/// Position of the current view in swipe order, and how many there are.
fn nav_position(app: &App) -> (usize, usize) {
    let order = app.nav_order();
    let i = order.iter().position(|v| *v == app.view).unwrap_or(0);
    (i, order.len())
}

/// What can change on screen now, each with a key of its content.
pub(crate) fn regions(app: &App, showing: Showing, env: &Env) -> Vec<Region> {
    match showing {
        Showing::Calibrate => views::setup::calibrate_regions(app),
        Showing::WifiSetup => views::setup::wifi_regions(env),
        Showing::Menu => views::menu::regions(app, env),
        Showing::View(view) => {
            let mut regions = match view {
                ViewId::Home => views::home::regions(app, env),
                ViewId::Forecast => views::forecast::regions(app, env),
                ViewId::Calendar => views::calendar::regions(app, env),
                ViewId::Screen(slot) => views::custom::regions(app, slot, env),
            };
            let (i, n) = nav_position(app);
            regions.push(views::nav_region(i, n));
            regions
        }
    }
}

/// Draws the whole screen; the canvas clips it to what is being repainted.
pub(crate) fn paint(app: &App, showing: Showing, c: &mut Canvas<'_>, env: &Env, theme: &Theme) {
    gfx::fill(c, views::area(), theme.bg);
    match showing {
        Showing::Calibrate => views::setup::paint_calibrate(app, c, theme),
        Showing::WifiSetup => views::setup::paint_wifi(c, env, theme),
        Showing::Menu => views::menu::paint(app, c, env, theme),
        Showing::View(view) => {
            match view {
                ViewId::Home => views::home::paint(app, c, env, theme),
                ViewId::Forecast => views::forecast::paint(app, c, env, theme),
                ViewId::Calendar => views::calendar::paint(app, c, env, theme),
                ViewId::Screen(slot) => views::custom::paint(app, slot, c, env, theme),
            }
            let (i, n) = nav_position(app);
            views::paint_nav(c, i, n, theme);
        }
    }
}
