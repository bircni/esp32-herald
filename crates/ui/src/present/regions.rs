use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};

use crate::{
    app::{App, Showing},
    gfx,
    model::Env,
};

/// Repaints what changed since the last call. The first call, a change of
/// screen and a change of theme repaint everything.
pub struct RegionPresenter {
    buf: Vec<Rgb565>,
    /// Keys of the regions as last drawn.
    cache: Vec<u64>,
    cache_screen: Option<(Showing, i64)>,
}

impl Default for RegionPresenter {
    fn default() -> Self {
        Self {
            buf: vec![Rgb565::BLACK; gfx::BAND_PIXELS],
            cache: Vec::new(),
            cache_screen: None,
        }
    }
}

impl RegionPresenter {
    /// Makes the next [`RegionPresenter::draw`] repaint the whole screen,
    /// e.g. after the display was re-initialised or turned.
    pub const fn invalidate(&mut self) {
        self.cache_screen = None;
    }

    /// Draws what changed on `display`.
    ///
    /// # Errors
    /// When the display fails; the next call repaints what is missing.
    pub fn draw<D>(&mut self, app: &mut App, display: &mut D, env: &Env) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        if app.is_asleep() {
            return Ok(());
        }
        if app.take_refresh() {
            self.invalidate();
        }
        let layout = app.layout();
        let showing = app.showing(env);
        let theme = layout.theme(app);
        let area = layout.area();
        // Layouts without regions are always drawn whole.
        let regions = layout.regions(app, showing, env).unwrap_or_default();
        let cache_id = Some((showing, theme.key()));
        let full = regions.is_empty()
            || self.cache_screen != cache_id
            || self.cache.len() != regions.len();

        let app: &App = app;
        let mut paint = |rect: Rectangle| {
            gfx::render(display, &mut self.buf, rect, |c| {
                layout.paint(app, c, env, &theme);
            })
        };
        let result = if full {
            paint(area)
        } else {
            regions
                .iter()
                .zip(&self.cache)
                .filter(|(region, cached)| region.key != **cached)
                .try_for_each(|(region, _)| paint(region.rect))
        };
        result?;
        self.cache = regions.iter().map(|r| r.key).collect();
        self.cache_screen = cache_id;
        Ok(())
    }
}
