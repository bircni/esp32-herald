//! The 5.79" e-paper panel: two SSD1683 controllers in cascade, each
//! driving one half of the 792 pixel rows. Commands with the top bit set go
//! to the second controller, which scans its half mirrored.
//!
//! Sequence and parameters follow Elecrow's example code and the `ESPHome`
//! driver for this panel.

use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use esp_idf_svc::hal::{
    delay::FreeRtos,
    gpio::{Input, Output, PinDriver},
    spi::{SpiDeviceDriver, SpiDriver},
};
use ui::present::{
    Refresh,
    panel::{SSD1683_CASCADE_ROW_BYTES as PANEL_ROW_BYTES, SSD1683_CASCADE_ROWS as PANEL_ROWS},
};

const SECOND: u8 = 0x80;

const SOFT_RESET: u8 = 0x12;
const DATA_ENTRY: u8 = 0x11;
const SET_X_RANGE: u8 = 0x44;
const SET_Y_RANGE: u8 = 0x45;
const SET_X: u8 = 0x4E;
const SET_Y: u8 = 0x4F;
const BORDER: u8 = 0x3C;
const UPDATE_CTRL1: u8 = 0x21;
const UPDATE_SEQ: u8 = 0x22;
const UPDATE: u8 = 0x20;
const WRITE_NEW: u8 = 0x24;
const WRITE_OLD: u8 = 0x26;

/// Each controller takes 50 bytes of a row; they share the middle byte.
const HALF: usize = PANEL_ROW_BYTES.div_ceil(2);
const SECOND_START: usize = PANEL_ROW_BYTES / 2;
/// Last byte column of a controller (0x31 = 49).
const LAST_X: u8 = (HALF - 1) as u8;
const LAST_Y: u16 = PANEL_ROWS as u16 - 1;

const BUSY_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Epd<'d> {
    spi: SpiDeviceDriver<'d, SpiDriver<'d>>,
    dc: PinDriver<'d, Output>,
    reset: PinDriver<'d, Output>,
    busy: PinDriver<'d, Input>,
    /// What the panel shows, for the partial refresh to compare against.
    shown: Option<Vec<u8>>,
    /// One controller's half of the picture, reused.
    half: Vec<u8>,
}

impl<'d> Epd<'d> {
    pub fn new(
        spi: SpiDeviceDriver<'d, SpiDriver<'d>>,
        dc: PinDriver<'d, Output>,
        reset: PinDriver<'d, Output>,
        busy: PinDriver<'d, Input>,
    ) -> Result<Self> {
        let mut epd = Self {
            spi,
            dc,
            reset,
            busy,
            shown: None,
            half: vec![0; HALF * PANEL_ROWS],
        };
        epd.init()?;
        Ok(epd)
    }

    fn init(&mut self) -> Result<()> {
        self.reset.set_high()?;
        FreeRtos::delay_ms(100);
        self.reset.set_low()?;
        FreeRtos::delay_ms(10);
        self.reset.set_high()?;
        FreeRtos::delay_ms(10);
        self.command(SOFT_RESET, &[])?;
        FreeRtos::delay_ms(10);
        self.wait()?;
        let [y_lo, y_hi] = LAST_Y.to_le_bytes();
        // First controller: left to right; second: right to left.
        self.command(DATA_ENTRY, &[0x03])?;
        self.command(SET_X_RANGE, &[0x00, LAST_X])?;
        self.command(SET_Y_RANGE, &[0x00, 0x00, y_lo, y_hi])?;
        self.command(DATA_ENTRY | SECOND, &[0x02])?;
        self.command(SET_X_RANGE | SECOND, &[LAST_X, 0x00])?;
        self.command(SET_Y_RANGE | SECOND, &[0x00, 0x00, y_lo, y_hi])?;
        self.wait()
    }

    fn command(&mut self, cmd: u8, data: &[u8]) -> Result<()> {
        self.dc.set_low()?;
        self.spi.write(&[cmd])?;
        if !data.is_empty() {
            self.dc.set_high()?;
            self.spi.write(data)?;
        }
        Ok(())
    }

    /// Waits while the panel is busy (busy pin high).
    fn wait(&self) -> Result<()> {
        let start = Instant::now();
        while self.busy.is_high() {
            if start.elapsed() > BUSY_TIMEOUT {
                bail!("e-paper stays busy");
            }
            FreeRtos::delay_ms(5);
        }
        Ok(())
    }

    fn home_cursors(&mut self) -> Result<()> {
        self.command(SET_X, &[0x00])?;
        self.command(SET_Y, &[0x00, 0x00])?;
        self.command(SET_X | SECOND, &[LAST_X])?;
        self.command(SET_Y | SECOND, &[0x00, 0x00])
    }

    /// Writes `frame` (in the panel's layout) to one of the two memories
    /// of both controllers.
    fn write(&mut self, memory: u8, frame: &[u8]) -> Result<()> {
        for (cmd, start) in [(memory, 0), (memory | SECOND, SECOND_START)] {
            for (row, out) in self.half.chunks_exact_mut(HALF).enumerate() {
                let from = row * PANEL_ROW_BYTES + start;
                out.copy_from_slice(&frame[from..from + HALF]);
            }
            let half = std::mem::take(&mut self.half);
            let result = self.command(cmd, &half);
            self.half = half;
            result?;
        }
        Ok(())
    }

    /// Shows `frame`, laid out as [`ui::present::panel::ssd1683_cascade`] returns
    /// it. Blocks until the panel is done (about 0.6 s partial, 3 s full).
    pub fn show(&mut self, frame: &[u8], refresh: Refresh) -> Result<()> {
        let full = refresh == Refresh::Full || self.shown.is_none();
        self.wait()?;
        self.command(BORDER, &[if full { 0x05 } else { 0x80 }])?;
        self.command(UPDATE_CTRL1, &[if full { 0x40 } else { 0x00 }, 0x10])?;
        self.home_cursors()?;
        // A partial refresh drives only the pixels that differ from the
        // "old" memory, so that must hold what the panel shows.
        let old = if full { None } else { self.shown.take() };
        self.write(WRITE_OLD, old.as_deref().unwrap_or(frame))?;
        self.home_cursors()?;
        self.write(WRITE_NEW, frame)?;
        self.command(UPDATE_SEQ, &[if full { 0xF7 } else { 0xFF }])?;
        self.command(UPDATE, &[])?;
        FreeRtos::delay_ms(10);
        let started = Instant::now();
        let result = self.wait();
        log::info!(
            "e-paper: {} refresh in {} ms",
            if full { "full" } else { "partial" },
            started.elapsed().as_millis()
        );
        self.shown = Some(frame.to_vec());
        result
    }
}
