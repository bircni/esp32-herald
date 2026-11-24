//! Backlight, speaker and RGB LED.

use anyhow::Result;
use esp_idf_svc::hal::{
    gpio::{Output, PinDriver},
    ledc::LedcDriver,
};
use ui::{Beep, Led};

pub struct Backlight<'d> {
    pwm: LedcDriver<'d>,
    percent: Option<u8>,
}

impl<'d> Backlight<'d> {
    pub const fn new(pwm: LedcDriver<'d>) -> Self {
        Self { pwm, percent: None }
    }

    pub fn set(&mut self, percent: u8) {
        if self.percent == Some(percent) {
            return;
        }
        // Perceived brightness is roughly the square root of the duty cycle.
        let p = u32::from(percent.min(100));
        let duty = self.pwm.get_max_duty() * p * p / 10_000;
        if let Err(e) = self.pwm.set_duty(duty) {
            log::warn!("backlight: {e}");
        }
        self.percent = Some(percent);
    }
}

/// (sound, silence) in milliseconds.
type Pattern = &'static [(u64, u64)];
const NOTICE: Pattern = &[(120, 100), (120, 0)];

/// Plays beeps as a square wave. `update` must be called regularly.
pub struct Speaker<'d> {
    pwm: LedcDriver<'d>,
    /// Amplifier shutdown pin: low switches the amplifier on.
    amp_off: PinDriver<'d, Output>,
    pattern: Pattern,
    step: usize,
    step_start_ms: u64,
    sounding: bool,
}

impl<'d> Speaker<'d> {
    pub fn new(pwm: LedcDriver<'d>, mut amp_off: PinDriver<'d, Output>) -> Result<Self> {
        amp_off.set_high()?;
        Ok(Self {
            pwm,
            amp_off,
            pattern: &[],
            step: 0,
            step_start_ms: 0,
            sounding: false,
        })
    }

    pub const fn play(&mut self, beep: Beep, now_ms: u64) {
        self.pattern = match beep {
            Beep::Notice => NOTICE,
        };
        self.step = 0;
        self.step_start_ms = now_ms;
    }

    fn sound(&mut self, on: bool) {
        if on == self.sounding {
            return;
        }
        self.sounding = on;
        let duty = if on { self.pwm.get_max_duty() / 2 } else { 0 };
        let result = self.pwm.set_duty(duty).and_then(|()| {
            if on {
                self.amp_off.set_low()
            } else {
                self.amp_off.set_high()
            }
        });
        if let Err(e) = result {
            log::warn!("speaker: {e}");
        }
    }

    pub fn update(&mut self, now_ms: u64) {
        let Some(&(on_ms, off_ms)) = self.pattern.get(self.step) else {
            self.sound(false);
            return;
        };
        let elapsed = now_ms.saturating_sub(self.step_start_ms);
        if elapsed >= on_ms + off_ms {
            self.step += 1;
            self.step_start_ms = now_ms;
            self.sound(false);
        } else {
            self.sound(elapsed < on_ms);
        }
    }
}

pub struct RgbLed<'d> {
    /// Red, green, blue. The LED is common anode: low lights a channel.
    pins: [PinDriver<'d, Output>; 3],
    shown: Option<Led>,
}

impl<'d> RgbLed<'d> {
    pub const fn new(pins: [PinDriver<'d, Output>; 3]) -> Self {
        Self { pins, shown: None }
    }

    pub fn set(&mut self, led: Led) {
        if self.shown == Some(led) {
            return;
        }
        let channels: [bool; 3] = led.channels().into();
        for (pin, lit) in self.pins.iter_mut().zip(channels) {
            let result = if lit { pin.set_low() } else { pin.set_high() };
            if let Err(e) = result {
                log::warn!("led: {e}");
            }
        }
        self.shown = Some(led);
    }
}
