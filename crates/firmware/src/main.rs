//! Herald firmware for the Freenove ESP32 Display 3.5" (FNK0103).
//!
//! The main task runs the UI loop: read touch, advance the app, repaint
//! what changed, drive backlight, LED and speaker. Networking lives on its
//! own thread (see `net`).

mod board;
mod logger;
mod mqtt;
mod net;
mod outputs;
mod shared;
mod storage;
mod touch;

use std::{sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use embedded_graphics::{pixelcolor::Rgb565, prelude::DrawTarget};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{
        delay::FreeRtos,
        gpio::{Gpio4, Gpio26, Gpio27, PinDriver},
        ledc::{LEDC, LedcDriver, LedcTimerDriver, Resolution, config::TimerConfig},
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
use mipidsi::{
    Builder,
    interface::SpiInterface,
    models::ST7796,
    options::{ColorInversion, ColorOrder, Orientation},
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
    logger::init();
    log::info!("herald {} starting", ui::VERSION);

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
    let touch = Touch::new(touch_spi);

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

    let (backlight, speaker) = pwm_outputs(p.ledc, p.pins.gpio27, p.pins.gpio26, p.pins.gpio4)?;
    let led = RgbLed::new([
        PinDriver::output(p.pins.gpio22)?,
        PinDriver::output(p.pins.gpio16)?,
        PinDriver::output(p.pins.gpio17)?,
    ]);

    let settings = storage.settings().unwrap_or_default();
    let timezone = settings.timezone.clone();
    shared::set_timezone(&timezone);
    let calibration = storage.calibration();
    #[cfg(feature = "qemu")]
    let calibration = calibration.or(Some(ui::Calibration::identity()));
    let app = App::new(settings, calibration);
    let shared = Arc::new(Shared::new(app, storage, net::running_slot()));

    if cfg!(feature = "qemu") {
        log::warn!("built for QEMU: networking is disabled");
        drop((p.modem, sysloop, nvs));
    } else {
        net::spawn(Arc::clone(&shared), p.modem, sysloop, nvs)?;
    }

    let mut ui = UiLoop {
        shared,
        touch,
        backlight,
        speaker,
        led,
        timezone,
        next_log_ms: 0,
        frames: 0,
        busy_ms: 0,
        worst_ms: 0,
    };
    loop {
        let started = std::time::Instant::now();
        ui.step(&mut display);
        let busy = started.elapsed();
        ui.count(busy);
        let rest = TICK.saturating_sub(busy);
        // Always yield for at least a tick so lower-priority tasks run.
        FreeRtos::delay_ms(rest.as_millis().max(1) as u32);
    }
}

/// Backlight dimming and the beeper, both driven by LEDC PWM.
fn pwm_outputs(
    ledc: LEDC,
    backlight_pin: Gpio27<'static>,
    tone_pin: Gpio26<'static>,
    amp_pin: Gpio4<'static>,
) -> Result<(Backlight<'static>, Speaker<'static>)> {
    let backlight_timer = LedcTimerDriver::new(
        ledc.timer0,
        &TimerConfig::new()
            .frequency(Hertz(5_000))
            .resolution(Resolution::Bits10),
    )?;
    let backlight = Backlight::new(LedcDriver::new(
        ledc.channel0,
        backlight_timer,
        backlight_pin,
    )?);
    // A 2 kHz square wave is plenty for beeps and needs no sample buffer.
    let tone_timer = LedcTimerDriver::new(
        ledc.timer1,
        &TimerConfig::new()
            .frequency(Hertz(2_000))
            .resolution(Resolution::Bits10),
    )?;
    let speaker = Speaker::new(
        LedcDriver::new(ledc.channel1, tone_timer, tone_pin)?,
        PinDriver::output(amp_pin)?,
    )?;
    Ok((backlight, speaker))
}

/// The UI side: touch in, app, display, backlight, LED and speaker out.
struct UiLoop<'d> {
    shared: Arc<Shared>,
    touch: Touch<'d>,
    backlight: Backlight<'d>,
    speaker: Speaker<'d>,
    led: RgbLed<'d>,
    timezone: String,
    next_log_ms: u64,
    /// Frames and busy time since the last log line.
    frames: u64,
    busy_ms: u64,
    worst_ms: u64,
}

impl UiLoop<'_> {
    fn step<D>(&mut self, display: &mut D)
    where
        D: DrawTarget<Color = Rgb565>,
        D::Error: core::fmt::Debug,
    {
        let raw = self.touch.read();
        let env = self.shared.env();
        let mut app = self.shared.app();
        app.tick(&env, raw);
        self.shared.apply_effects(&mut app);
        if let Err(e) = app.draw(display, &env) {
            log::warn!("display: {e:?}");
            app.invalidate();
        }
        let (backlight, led) = (app.backlight(), app.led());
        let timezone_changed = app.settings().timezone != self.timezone;
        if timezone_changed {
            self.timezone.clone_from(&app.settings().timezone);
        }
        drop(app);

        self.backlight.set(backlight);
        self.led.set(led);
        if timezone_changed {
            shared::set_timezone(&self.timezone);
        }
        for beep in self.shared.take_beeps() {
            self.speaker.play(beep, env.now_ms);
        }
        self.speaker.update(env.now_ms);
        if self.shared.restart_due() {
            log::info!("restarting");
            reset::restart();
        }
        if env.now_ms >= self.next_log_ms {
            self.next_log_ms = env.now_ms + HEAP_LOG_INTERVAL_MS;
            let (free, min) = shared::heap_stats();
            log::info!(
                "heap: {free} bytes free, {min} at the lowest point; ui: {} frames, \
                 {} ms average, {} ms worst; {}",
                self.frames,
                self.busy_ms / self.frames.max(1),
                self.worst_ms,
                net::HTTP_STATS.take(),
            );
            (self.frames, self.busy_ms, self.worst_ms) = (0, 0, 0);
        }
    }

    fn count(&mut self, busy: Duration) {
        let ms = busy.as_millis() as u64;
        self.frames += 1;
        self.busy_ms += ms;
        self.worst_ms = self.worst_ms.max(ms);
    }
}
