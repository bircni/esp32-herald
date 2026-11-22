//! XPT2046 resistive touch controller on the display's SPI bus.

use esp_idf_svc::hal::spi::{SpiDeviceDriver, SpiDriver};
use ui::RawTouch;

use crate::board::TOUCH_PRESSURE_MIN;

// Control bytes: start bit, channel, 12-bit, differential. The low two bits
// keep the ADC powered between conversions (0b01) or power it down (0b00).
const Z1: u8 = 0xB1;
const Z2: u8 = 0xC1;
const A: u8 = 0x91;
const B: u8 = 0xD1;
const B_POWER_DOWN: u8 = 0xD0;

/// Z1 below this means nothing is touching the panel.
const Z1_MIN: u16 = 20;

/// Two readings of an axis further apart than this are treated as noise.
const MAX_JITTER: u16 = 120;

pub struct Touch<'d> {
    spi: SpiDeviceDriver<'d, &'d SpiDriver<'d>>,
}

impl<'d> Touch<'d> {
    pub const fn new(spi: SpiDeviceDriver<'d, &'d SpiDriver<'d>>) -> Self {
        Self { spi }
    }

    /// Returns the current touch position in raw ADC units, or `None` when
    /// nothing presses firmly enough or the reading is unstable.
    pub fn read(&mut self) -> Option<RawTouch> {
        // Each conversion takes 16 clocks: the result of one command arrives
        // while the next command byte is clocked out.
        const COMMANDS: [u8; 8] = [Z1, Z2, A, A, B, A, B, B_POWER_DOWN];
        let mut tx = [0u8; COMMANDS.len() * 2 + 1];
        for (i, cmd) in COMMANDS.into_iter().enumerate() {
            tx[i * 2] = cmd;
        }
        let mut rx = [0u8; COMMANDS.len() * 2 + 1];
        self.spi.transfer(&mut rx, &tx).ok()?;
        let value = |i: usize| (u16::from(rx[i * 2 + 1]) << 8 | u16::from(rx[i * 2 + 2])) >> 3;

        // Untouched, Z1 reads near zero and Z2 near full scale. A bus that
        // returns only zeros (no controller answering) must not look like a
        // firm press, hence the separate check on Z1.
        let (z1, z2) = (value(0), value(1));
        let pressure = (z1 + 4095).saturating_sub(z2);
        if z1 < Z1_MIN || pressure < TOUCH_PRESSURE_MIN {
            return None;
        }
        // The first reading of the A axis (index 2) settles the panel and is
        // discarded; the remaining ones are a1, b1, a2, b2, b3.
        let (a1, b1, a2, b2) = (value(3), value(4), value(5), value(6));
        if a1.abs_diff(a2) > MAX_JITTER || b1.abs_diff(b2) > MAX_JITTER {
            return None;
        }
        Some(RawTouch {
            x: u16::midpoint(a1, a2),
            y: u16::midpoint(b1, b2),
        })
    }
}
