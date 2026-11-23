//! Layouts, one per screen size class. A board's [`crate::Profile`] picks
//! one; a board with a new screen size needs a new layout here, nothing
//! else in the app.

pub mod landscape;
mod paper;
pub mod portrait;
pub mod wide;

use embedded_graphics::{prelude::*, primitives::Rectangle};

use crate::{
    app::{App, Showing},
    gfx::Canvas,
    model::{Env, ThemeKind, ViewId},
    profile::Profile,
    theme::Theme,
    views::{self, Region},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// 480x320 colour: views to swipe between, a menu, touch calibration.
    Landscape,
    /// 272x792 upright: one dashboard and custom screens as pages.
    Portrait,
    /// 792x272: the dashboard in three columns, custom screens as pages.
    Wide,
}

impl Layout {
    /// The layout for a screen: tall ones get the portrait layout, low and
    /// wide ones the wide layout, the rest the landscape one.
    #[must_use]
    pub const fn for_profile(screen: &Profile) -> Self {
        if screen.height > screen.width {
            Self::Portrait
        } else if screen.height < 300 {
            Self::Wide
        } else {
            Self::Landscape
        }
    }

    /// The size this layout draws.
    #[must_use]
    pub const fn area(self) -> Rectangle {
        match self {
            Self::Landscape => views::area(),
            Self::Portrait => Rectangle::new(
                Point::zero(),
                Size::new(portrait::W as u32, portrait::H as u32),
            ),
            Self::Wide => Rectangle::new(Point::zero(), Size::new(wide::W as u32, wide::H as u32)),
        }
    }

    /// Whether the layout has an on-screen menu (opened by a long press).
    #[must_use]
    pub const fn has_menu(self) -> bool {
        matches!(self, Self::Landscape)
    }

    /// The pages `Next` and `Previous` step through, and the current one.
    pub(crate) fn pages(self, app: &App) -> (Vec<ViewId>, ViewId) {
        match self {
            Self::Landscape => (app.nav_order(), app.view),
            Self::Portrait | Self::Wide => (paper::pages(app), paper::page(app)),
        }
    }

    /// Colours: the settings' theme on colour screens; black on white
    /// (white on black with the dark theme) on black-and-white ones. The
    /// e-paper layouts are drawn for black and white.
    pub(crate) fn theme(self, app: &App) -> Theme {
        let mono = match self {
            Self::Landscape => !app.profile().color,
            Self::Portrait | Self::Wide => true,
        };
        if mono {
            Theme::mono(app.settings.theme == ThemeKind::Dark)
        } else {
            app.theme()
        }
    }

    /// Draws the whole screen into `c`, which clips it to its area.
    pub(crate) fn paint(self, app: &App, c: &mut Canvas<'_>, env: &Env, theme: &Theme) {
        let showing = app.showing(env);
        match self {
            Self::Landscape => landscape::paint(app, showing, c, env, theme),
            Self::Portrait => {
                crate::gfx::fill(c, self.area(), theme.bg);
                portrait::paint(app, c, env, theme);
            }
            Self::Wide => {
                crate::gfx::fill(c, self.area(), theme.bg);
                wide::paint(app, c, env, theme);
            }
        }
    }

    /// What can change, for region redraws. `None` for layouts that are
    /// only ever drawn whole.
    pub(crate) fn regions(self, app: &App, showing: Showing, env: &Env) -> Option<Vec<Region>> {
        match self {
            Self::Landscape => Some(landscape::regions(app, showing, env)),
            Self::Portrait | Self::Wide => None,
        }
    }
}
