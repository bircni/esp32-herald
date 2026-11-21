//! Deskbuddy firmware for the Freenove ESP32 Display 3.5" (FNK0103).
//!
//! The main task runs the UI loop: read touch, advance the app, repaint
//! what changed, drive backlight, LED and speaker. Networking lives on its
//! own thread (see `net`).

mod board;
mod net;
mod outputs;
mod shared;
mod storage;
mod touch;

use std::{sync::Arc, time::Duration};

use anyhow::{anyhow, Result};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{
        delay::FreeRtos,
        gpio::PinDriver,
        ledc::{config::TimerConfig, LedcDriver, LedcTimerDriver, Resolution},
        peripherals::Peripherals,
        reset,
        spi::{
            config::{Config as SpiConfig, DriverConfig},
            Dma, SpiDeviceDriver, SpiDriver,
        },
        units::Hertz,
    },
    nvs::EspDefaultNvsPartition,
    sys,
};
use mipidsi::{
    interface::SpiInterface,
    models::ST7796,
    options::{ColorInversion, ColorOrder, Orientation},
    Builder,
};
use ui::App;

use crate::{
    outputs::{Backlight, RgbLed, Speaker},
    shared::Shared,
    storage::Storage,
    touch::Touch,
};

/// One UI step. Also the touch sampling interval the gesture code expects.
const TICK: Duration = Duration::from_millis(30);
const HEAP_LOG_INTERVAL_MS: u64 = 60_000;
/// Native size of the panel, before rotation.
const PANEL_W: u16 = 320;
const PANEL_H: u16 = 480;

fn main() -> Result<()> {
    sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("deskbuddy {} starting", ui::VERSION);

    let p = Peripherals::take()?;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let storage = Storage::new(nvs.clone())?;

    // Display and touch share one SPI bus. The bus lives for the whole
    // program, so it is leaked to get a 'static reference for both devices.
    // Without DMA every transfer is limited to 64 bytes, which made a full
    // repaint take over a second.
    let spi: &'static SpiDriver<'static> = Box::leak(Box::new(SpiDriver::new(
        p.spi2,
        p.pins.gpio14,
        p.pins.gpio13,
        Some(p.pins.gpio12),
        &DriverConfig::new().dma(Dma::Auto(4096)),
    )?));
    let lcd_spi = SpiDeviceDriver::new(
        spi,
        Some(p.pins.gpio15),
        &SpiConfig::new().baudrate(Hertz(board::LCD_SPI_HZ)),
    )?;
    let touch_spi = SpiDeviceDriver::new(
        spi,
        Some(p.pins.gpio33),
        &SpiConfig::new().baudrate(Hertz(board::TOUCH_SPI_HZ)),
    )?;
    let mut touch = Touch::new(touch_spi);

    let dc = PinDriver::output(p.pins.gpio2)?;
    let spi_buffer: &'static mut [u8] = Box::leak(vec![0u8; 4096].into_boxed_slice());
    let mut display = Builder::new(ST7796, SpiInterface::new(lcd_spi, dc, spi_buffer))
        .display_size(PANEL_W, PANEL_H)
        .orientation(Orientation {
            rotation: board::LCD_ROTATION,
            mirrored: board::LCD_MIRRORED,
        })
        .color_order(if board::LCD_BGR {
            ColorOrder::Bgr
        } else {
            ColorOrder::Rgb
        })
        .invert_colors(if board::LCD_INVERTED {
            ColorInversion::Inverted
        } else {
            ColorInversion::Normal
        })
        .init(&mut FreeRtos)
        .map_err(|e| anyhow!("display init failed: {e:?}"))?;

    let backlight_timer = LedcTimerDriver::new(
        p.ledc.timer0,
        &TimerConfig::new()
            .frequency(Hertz(5_000))
            .resolution(Resolution::Bits10),
    )?;
    let mut backlight = Backlight::new(LedcDriver::new(
        p.ledc.channel0,
        backlight_timer,
        p.pins.gpio27,
    )?);

    // A 2 kHz square wave is plenty for beeps and needs no sample buffer.
    let tone_timer = LedcTimerDriver::new(
        p.ledc.timer1,
        &TimerConfig::new()
            .frequency(Hertz(2_000))
            .resolution(Resolution::Bits10),
    )?;
    let mut speaker = Speaker::new(
        LedcDriver::new(p.ledc.channel1, tone_timer, p.pins.gpio26)?,
        PinDriver::output(p.pins.gpio4)?,
    )?;
    let mut led = RgbLed::new([
        PinDriver::output(p.pins.gpio22)?,
        PinDriver::output(p.pins.gpio16)?,
        PinDriver::output(p.pins.gpio17)?,
    ]);

    let settings = storage.settings().unwrap_or_default();
    let mut timezone = settings.timezone.clone();
    shared::set_timezone(&timezone);
    let calibration = storage.calibration();
    #[cfg(feature = "qemu")]
    let calibration = calibration.or(Some(ui::Calibration::identity()));
    let app = App::new(settings, calibration);
    let shared = Arc::new(Shared::new(app, storage));

    if cfg!(feature = "qemu") {
        log::warn!("built for QEMU: networking is disabled");
        drop((p.modem, sysloop, nvs));
    } else {
        net::spawn(Arc::clone(&shared), p.modem, sysloop, nvs)?;
    }

    let mut next_heap_log_ms = 0;
    // Busy time of the UI loop since the last log line.
    let (mut frames, mut busy_ms, mut worst_ms) = (0u64, 0u64, 0u64);
    loop {
        let frame_start = std::time::Instant::now();
        let raw = touch.read();
        let env = shared.env();
        {
            let mut app = shared.app();
            app.tick(&env, raw);
            shared.apply_effects(&mut app);
            if let Err(e) = app.draw(&mut display, &env) {
                log::warn!("display: {e:?}");
                app.invalidate();
            }
            backlight.set(app.backlight());
            led.set(app.led());
            if app.settings().timezone != timezone {
                timezone = app.settings().timezone.clone();
                shared::set_timezone(&timezone);
            }
        }

        for beep in shared.take_beeps() {
            speaker.play(beep, env.now_ms);
        }
        speaker.update(env.now_ms);

        if shared.restart_due() {
            log::info!("restarting");
            reset::restart();
        }
        if env.now_ms >= next_heap_log_ms {
            next_heap_log_ms = env.now_ms + HEAP_LOG_INTERVAL_MS;
            // SAFETY: plain getters without preconditions.
            let (free, min) = unsafe {
                (
                    sys::esp_get_free_heap_size(),
                    sys::esp_get_minimum_free_heap_size(),
                )
            };
            log::info!(
                "heap: {free} bytes free, {min} at the lowest point; ui: {} frames, \
                 {} ms average, {worst_ms} ms worst",
                frames,
                busy_ms / frames.max(1),
            );
            (frames, busy_ms, worst_ms) = (0, 0, 0);
        }

        let busy = frame_start.elapsed();
        frames += 1;
        busy_ms += busy.as_millis() as u64;
        worst_ms = worst_ms.max(busy.as_millis() as u64);
        let rest = TICK.saturating_sub(busy);
        // Always yield for at least a tick so lower-priority tasks run.
        FreeRtos::delay_ms(rest.as_millis().max(1) as u32);
    }
}
