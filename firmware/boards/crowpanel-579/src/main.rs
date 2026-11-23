//! Herald on the Elecrow `CrowPanel` ESP32 5.79" e-paper display, used
//! upright: a black-and-white panel, a rotary switch and two buttons.
//!
//! About once a second, and right after a button, the picture is drawn;
//! when it differs from what the panel shows, the panel refreshes.

mod board;
mod buttons;
mod epd;

use std::time::{Duration, Instant};

use anyhow::Result;
use esp_idf_svc::hal::{
    delay::FreeRtos,
    gpio::{AnyIOPin, PinDriver, Pull},
    spi::{
        Dma, SpiDeviceDriver, SpiDriver,
        config::{Config as SpiConfig, DriverConfig},
    },
    units::Hertz,
};
use herald_esp::Shared;
use ui::{
    App, Env, Profile,
    epaper::{self, Button, Refresher, Renderer, Response},
};

use crate::{buttons::Buttons, epd::Epd};

/// How often the picture is drawn to see whether it changed.
const RENDER_INTERVAL: Duration = Duration::from_secs(1);

fn main() -> Result<()> {
    let (p, start) = herald_esp::start(board::NAME)?;

    // The panel has its own power switch; it stays on for good.
    let mut power = PinDriver::output(p.pins.gpio7)?;
    power.set_high()?;
    core::mem::forget(power);
    FreeRtos::delay_ms(50);

    let spi = SpiDriver::new(
        p.spi2,
        p.pins.gpio12,
        p.pins.gpio11,
        None::<AnyIOPin<'static>>,
        &DriverConfig::new().dma(Dma::Auto(4096)),
    )?;
    let device = SpiDeviceDriver::new(
        spi,
        Some(p.pins.gpio45),
        &SpiConfig::new().baudrate(Hertz(board::EPD_SPI_HZ)),
    )?;
    let epd = Epd::new(
        device,
        PinDriver::output(p.pins.gpio46)?,
        PinDriver::output(p.pins.gpio47)?,
        PinDriver::input(p.pins.gpio48, Pull::Floating)?,
    )?;
    let buttons = Buttons::new([
        (PinDriver::input(p.pins.gpio1, Pull::Up)?, Button::Exit),
        (PinDriver::input(p.pins.gpio2, Pull::Up)?, Button::Menu),
        (PinDriver::input(p.pins.gpio4, Pull::Up)?, Button::Down),
        (PinDriver::input(p.pins.gpio5, Pull::Up)?, Button::Press),
        (PinDriver::input(p.pins.gpio6, Pull::Up)?, Button::Up),
    ]);

    let board = CrowPanel {
        epd,
        buttons,
        // The whole picture in one pass; the buffer goes to PSRAM.
        renderer: Renderer::new(epaper::H as usize),
        refresher: Refresher::default(),
        upside_down: false,
        next_render: Instant::now(),
    };
    herald_esp::run(board, p.modem, start)
}

struct CrowPanel {
    epd: Epd<'static>,
    buttons: Buttons<'static>,
    renderer: Renderer,
    refresher: Refresher,
    /// How the panel shows the picture now.
    upside_down: bool,
    next_render: Instant,
}

impl herald_esp::Board for CrowPanel {
    /// Button polling interval.
    const TICK: Duration = Duration::from_millis(20);
    const PROFILE: Profile = Profile::CROWPANEL_579;

    fn input(&mut self, app: &mut App, env: &Env) {
        app.tick(env, None);
        for b in self.buttons.poll() {
            if epaper::button(app, b, env) == Response::FullRefresh {
                self.refresher.force_full();
            }
            // Drawn right away after a button.
            self.next_render = Instant::now();
        }
    }

    fn present(&mut self, shared: &Shared, env: &Env) {
        if Instant::now() < self.next_render {
            return;
        }
        self.next_render = Instant::now() + RENDER_INTERVAL;
        let (frame, flip) = {
            let app = shared.app();
            (self.renderer.render(&app, env), app.settings().upside_down)
        };
        // The upright picture stays the same when it is turned.
        if flip != self.upside_down {
            self.upside_down = flip;
            self.refresher.force_full();
        }
        let Some(refresh) = self.refresher.next(&frame) else {
            return;
        };
        match self.epd.show(&frame.to_panel(self.upside_down), refresh) {
            Ok(()) => shared.app().set_shown_frame(frame),
            Err(e) => {
                log::warn!("e-paper: {e:#}");
                self.refresher.force_full();
            }
        }
    }
}
