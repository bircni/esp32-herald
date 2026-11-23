//! Start-up and the main loop, the same for every board.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{delay::FreeRtos, modem::WifiModemPeripheral, peripherals::Peripherals, reset},
    nvs::EspDefaultNvsPartition,
    sys,
};
use ui::{App, Beep, Calibration, Env, Led};

use crate::{
    logger, net,
    shared::{self, Shared},
    storage::Storage,
};

const STATS_INTERVAL_MS: u64 = 60_000;

/// One board: its display, input and outputs. The loop calls `input`,
/// carries out what the app asked for, then `present` and `outputs`.
pub trait Board {
    /// Time between loop steps (a step that takes longer starts the next
    /// one right away).
    const TICK: Duration;
    /// Whether the board has a touch panel. Without one there is no
    /// calibration screen.
    const TOUCH: bool;
    /// Whether to start `WiFi` (off for emulators without a radio).
    const NETWORK: bool = true;

    /// Reads the input and advances the app: calls [`App::tick`] once.
    fn input(&mut self, app: &mut App, env: &Env);

    /// Shows the app on the display. Locks the app itself, for as short as
    /// it can: an e-paper refresh may take seconds.
    fn present(&mut self, shared: &Shared, env: &Env);

    /// Backlight, LED and speaker. Boards without them ignore this.
    fn outputs(&mut self, out: &Outputs, now_ms: u64) {
        let _ = (out, now_ms);
    }
}

/// What the app wants from the board's outputs after a step.
pub struct Outputs {
    /// Backlight in percent; 0 is off.
    pub backlight: u8,
    pub led: Led,
    /// Beeps requested since the last step.
    pub beeps: Vec<Beep>,
}

/// What [`start`] sets up before the board takes its pins.
pub struct Start {
    sysloop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
    storage: Storage,
}

/// Starts the log and takes the peripherals. Call first thing in `main`,
/// then build the board from the peripherals' pins. `board` names it in the
/// log.
///
/// # Errors
/// When ESP-IDF's event loop or NVS cannot be set up.
pub fn start(board: &str) -> Result<(Peripherals, Start)> {
    sys::link_patches();
    logger::init();
    log::info!("herald {} on {board} starting", ui::VERSION);
    let peripherals = Peripherals::take()?;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let storage = Storage::new(nvs.clone())?;
    Ok((
        peripherals,
        Start {
            sysloop,
            nvs,
            storage,
        },
    ))
}

/// Loads the settings, starts the network and runs the main loop. Only
/// returns when the network cannot be started.
///
/// # Errors
/// When the network thread cannot be created.
pub fn run<B, M>(mut board: B, modem: M, start: Start) -> Result<()>
where
    B: Board,
    M: WifiModemPeripheral + Send + 'static,
{
    let Start {
        sysloop,
        nvs,
        storage,
    } = start;
    let settings = storage.settings().unwrap_or_default();
    let mut timezone = settings.timezone.clone();
    shared::set_timezone(&timezone);
    // Without a touch panel the identity calibration skips the calibration
    // screen.
    let calibration = if B::TOUCH {
        storage.calibration()
    } else {
        Some(Calibration::identity())
    };
    let app = App::new(settings, calibration);
    let shared = Arc::new(Shared::new(app, storage, net::running_slot()));
    if B::NETWORK {
        net::spawn(Arc::clone(&shared), modem, sysloop, nvs)?;
    } else {
        log::warn!("networking is disabled");
        drop((modem, sysloop, nvs));
    }

    let mut stats = Stats::default();
    let mut next_stats_ms = STATS_INTERVAL_MS;
    loop {
        let started = Instant::now();
        let env = shared.env();
        let mut out = {
            let mut app = shared.app();
            board.input(&mut app, &env);
            shared.apply_effects(&mut app);
            if app.settings().timezone != timezone {
                timezone.clone_from(&app.settings().timezone);
                shared::set_timezone(&timezone);
            }
            Outputs {
                backlight: app.backlight(),
                led: app.led(),
                beeps: Vec::new(),
            }
        };
        board.present(&shared, &env);
        out.beeps = shared.take_beeps();
        board.outputs(&out, env.now_ms);

        if shared.restart_due() {
            log::info!("restarting");
            reset::restart();
        }
        let busy = started.elapsed();
        stats.count(busy);
        if env.now_ms >= next_stats_ms {
            next_stats_ms = env.now_ms + STATS_INTERVAL_MS;
            stats.log();
        }
        // Always yield for at least a tick so lower-priority tasks run.
        let rest = B::TICK.saturating_sub(busy);
        FreeRtos::delay_ms(rest.as_millis().max(1) as u32);
    }
}

/// Loop timing since the last log line.
#[derive(Default)]
struct Stats {
    steps: u64,
    busy_ms: u64,
    worst_ms: u64,
}

impl Stats {
    fn count(&mut self, busy: Duration) {
        let ms = busy.as_millis() as u64;
        self.steps += 1;
        self.busy_ms += ms;
        self.worst_ms = self.worst_ms.max(ms);
    }

    fn log(&mut self) {
        let (free, min) = shared::heap_stats();
        log::info!(
            "heap: {free} bytes free, {min} at the lowest point; ui: {} steps, \
             {} ms average, {} ms worst; {}",
            self.steps,
            self.busy_ms / self.steps.max(1),
            self.worst_ms,
            net::HTTP_STATS.take(),
        );
        *self = Self::default();
    }
}
