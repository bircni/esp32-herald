//! Herald firmware for the Elecrow `CrowPanel` ESP32 5.79" e-paper display,
//! used upright.
//!
//! The main task polls the buttons, advances the app and, about once a
//! second, draws the picture; when it differs from what the panel shows,
//! the panel refreshes. Networking, MQTT, the web app and storage are the
//! TFT firmware's own modules, shared through `#[path]`.

mod board;
mod buttons;
mod epd;
#[path = "../../firmware/src/logger.rs"]
mod logger;
#[path = "../../firmware/src/mqtt.rs"]
mod mqtt;
#[path = "../../firmware/src/net.rs"]
mod net;
#[path = "../../firmware/src/shared.rs"]
mod shared;
#[path = "../../firmware/src/storage.rs"]
#[expect(dead_code, reason = "the touch calibration is for the TFT board")]
mod storage;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{
        delay::FreeRtos,
        gpio::{PinDriver, Pull},
        peripherals::Peripherals,
        reset,
        spi::{
            Dma, SpiDeviceDriver, SpiDriver,
            config::{Config as SpiConfig, DriverConfig},
        },
        units::Hertz,
    },
    nvs::EspDefaultNvsPartition,
    sys,
};
use ui::{
    App, Calibration,
    epaper::{self, Button, Refresher, Renderer, Response},
};

use crate::{buttons::Buttons, epd::Epd, shared::Shared, storage::Storage};

/// Button polling interval.
const TICK: Duration = Duration::from_millis(20);
/// How often the picture is drawn to see whether it changed.
const RENDER_INTERVAL: Duration = Duration::from_secs(1);
const HEAP_LOG_INTERVAL: Duration = Duration::from_secs(60);

fn main() -> Result<()> {
    sys::link_patches();
    logger::init();
    log::info!("herald {} (e-paper) starting", ui::VERSION);

    let p = Peripherals::take()?;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let storage = Storage::new(nvs.clone())?;

    // The panel has its own power switch; it stays on for good.
    let mut power = PinDriver::output(p.pins.gpio7)?;
    power.set_high()?;
    core::mem::forget(power);
    FreeRtos::delay_ms(50);

    let spi = SpiDriver::new(
        p.spi2,
        p.pins.gpio12,
        p.pins.gpio11,
        None::<esp_idf_svc::hal::gpio::AnyIOPin<'static>>,
        &DriverConfig::new().dma(Dma::Auto(4096)),
    )?;
    let device = SpiDeviceDriver::new(
        spi,
        Some(p.pins.gpio45),
        &SpiConfig::new().baudrate(Hertz(board::EPD_SPI_HZ)),
    )?;
    let mut epd = Epd::new(
        device,
        PinDriver::output(p.pins.gpio46)?,
        PinDriver::output(p.pins.gpio47)?,
        PinDriver::input(p.pins.gpio48, Pull::Floating)?,
    )?;
    let mut buttons = Buttons::new([
        (PinDriver::input(p.pins.gpio1, Pull::Up)?, Button::Exit),
        (PinDriver::input(p.pins.gpio2, Pull::Up)?, Button::Menu),
        (PinDriver::input(p.pins.gpio4, Pull::Up)?, Button::Down),
        (PinDriver::input(p.pins.gpio5, Pull::Up)?, Button::Press),
        (PinDriver::input(p.pins.gpio6, Pull::Up)?, Button::Up),
    ]);

    let settings = storage.settings().unwrap_or_default();
    let mut timezone = settings.timezone.clone();
    shared::set_timezone(&timezone);
    // No touch panel: the identity calibration skips the calibration screen.
    let app = App::new(settings, Some(Calibration::identity()));
    let shared = Arc::new(Shared::new(app, storage, net::running_slot()));
    net::spawn(Arc::clone(&shared), p.modem, sysloop, nvs)?;

    // The whole picture in one pass; the buffer goes to PSRAM.
    let mut renderer = Renderer::new(epaper::H as usize);
    let mut refresher = Refresher::default();
    let mut upside_down = false;
    let mut next_render = Instant::now();
    let mut next_log = Instant::now() + HEAP_LOG_INTERVAL;
    loop {
        let env = shared.env();
        let pressed = buttons.poll();
        let frame = {
            let mut app = shared.app();
            app.tick(&env, None);
            for b in &pressed {
                if epaper::button(&mut app, *b, &env) == Response::FullRefresh {
                    refresher.force_full();
                }
            }
            shared.apply_effects(&mut app);
            // No speaker on this board.
            drop(shared.take_beeps());
            if app.settings().timezone != timezone {
                timezone.clone_from(&app.settings().timezone);
                shared::set_timezone(&timezone);
            }
            // Drawn right away after a button, else once a second.
            let due = !pressed.is_empty() || Instant::now() >= next_render;
            let frame = due.then(|| (renderer.render(&app, &env), app.settings().upside_down));
            drop(app);
            frame
        };
        if let Some((frame, flip)) = frame {
            next_render = Instant::now() + RENDER_INTERVAL;
            // The upright picture stays the same when it is turned.
            if flip != upside_down {
                upside_down = flip;
                refresher.force_full();
            }
            if let Some(refresh) = refresher.next(&frame) {
                match epd.show(&frame.to_panel(upside_down), refresh) {
                    Ok(()) => shared.app().set_shown_frame(frame),
                    Err(e) => {
                        log::warn!("e-paper: {e:#}");
                        refresher.force_full();
                    }
                }
            }
        }
        if shared.restart_due() {
            log::info!("restarting");
            reset::restart();
        }
        if Instant::now() >= next_log {
            next_log = Instant::now() + HEAP_LOG_INTERVAL;
            let (free, min) = shared::heap_stats();
            log::info!(
                "heap: {free} bytes free, {min} at the lowest point; {}",
                net::HTTP_STATS.take(),
            );
        }
        FreeRtos::delay_ms(TICK.as_millis() as u32);
    }
}
