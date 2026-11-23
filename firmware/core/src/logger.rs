//! The log goes to the serial console as before, and its last lines also
//! into `ui::logbuf`, so the web page and MQTT can show them.

use esp_idf_svc::log::{EspIdfLogFilter, EspIdfLogger};
use log::{Level, LevelFilter, Log, Metadata, Record};

struct Herald {
    serial: EspIdfLogger<EspIdfLogFilter>,
}

static LOGGER: Herald = Herald {
    serial: EspIdfLogger::new(EspIdfLogFilter::new()),
};

impl Log for Herald {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.serial.enabled(metadata)
    }

    fn log(&self, record: &Record<'_>) {
        self.serial.log(record);
        if !self.enabled(record.metadata()) {
            return;
        }
        let level = match record.level() {
            Level::Error => 'E',
            Level::Warn => 'W',
            Level::Info => 'I',
            Level::Debug => 'D',
            Level::Trace => 'V',
        };
        let line = format!("{level} {}: {}", record.target(), record.args());
        ui::logbuf::push(&line, record.level() <= Level::Warn);
    }

    fn flush(&self) {
        self.serial.flush();
    }
}

/// Installs the logger; call once, first thing.
pub fn init() {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(LevelFilter::Info);
    }
}
