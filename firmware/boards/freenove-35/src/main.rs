//! Herald on the Freenove ESP32 Display 3.5" (FNK0103): a landscape colour
//! TFT with resistive touch, backlight, RGB LED and speaker.

mod board;
mod outputs;
mod touch;

use std::time::Duration;

use anyhow::{Result, anyhow};
use embedded_graphics::{pixelcolor::Rgb565, prelude::DrawTarget};
use esp_idf_svc::hal::{
    delay::FreeRtos,
    gpio::{Gpio4, Gpio26, Gpio27, PinDriver},
    ledc::{LEDC, LedcDriver, LedcTimerDriver, Resolution, config::TimerConfig},
    spi::{
        Dma, SpiDeviceDriver, SpiDriver,
        config::{Config as SpiConfig, DriverConfig},
    },
    units::Hertz,
};
use herald_esp::{Outputs, Shared};
use mipidsi::{
    Builder,
    interface::SpiInterface,
    models::ST7796,
    options::{ColorInversion, ColorOrder, Orientation},
};
use ui::{App, Env};

use crate::{
    outputs::{Backlight, RgbLed, Speaker},
    touch::Touch,
};

/// Native size of the panel, before rotation.
const PANEL_W: u16 = 320;
const PANEL_H: u16 = 480;

fn main() -> Result<()> {
    let (p, start) = herald_esp::start(board::NAME)?;

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

    let dc = PinDriver::output(p.pins.gpio2)?;
    let spi_buffer: &'static mut [u8] = Box::leak(vec![0u8; 4096].into_boxed_slice());
    let display = Builder::new(ST7796, SpiInterface::new(lcd_spi, dc, spi_buffer))
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

    let board = Freenove {
        display,
        touch: Touch::new(touch_spi),
        backlight,
        speaker,
        led,
    };
    herald_esp::run(board, p.modem, start)
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

struct Freenove<D> {
    display: D,
    touch: Touch<'static>,
    backlight: Backlight<'static>,
    speaker: Speaker<'static>,
    led: RgbLed<'static>,
}

impl<D> herald_esp::Board for Freenove<D>
where
    D: DrawTarget<Color = Rgb565>,
    D::Error: core::fmt::Debug,
{
    /// Also the touch sampling interval the gesture code expects.
    const TICK: Duration = Duration::from_millis(30);
    // QEMU has neither a touch panel nor a WiFi radio.
    const TOUCH: bool = !cfg!(feature = "qemu");
    const NETWORK: bool = !cfg!(feature = "qemu");

    fn input(&mut self, app: &mut App, env: &Env) {
        app.tick(env, self.touch.read());
    }

    fn present(&mut self, shared: &Shared, env: &Env) {
        let mut app = shared.app();
        if let Err(e) = app.draw(&mut self.display, env) {
            log::warn!("display: {e:?}");
            app.invalidate();
        }
    }

    fn outputs(&mut self, out: &Outputs, now_ms: u64) {
        self.backlight.set(out.backlight);
        self.led.set(out.led);
        for beep in &out.beeps {
            self.speaker.play(*beep, now_ms);
        }
        self.speaker.update(now_ms);
    }
}
