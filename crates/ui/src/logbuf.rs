//! The last log lines in RAM, so the device can be debugged from the web
//! page and over MQTT instead of a serial cable. The platform's logger
//! feeds [`push`].

use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard, PoisonError},
};

/// Lines kept.
pub const LINES: usize = 80;
/// Longest line kept; longer ones are cut.
const LINE_MAX: usize = 200;
/// Warnings waiting for MQTT; more are dropped.
const PENDING_MAX: usize = 16;

struct Ring {
    lines: VecDeque<String>,
    /// Warnings and errors not yet published on `<prefix>/log`.
    pending: Vec<String>,
}

static RING: Mutex<Ring> = Mutex::new(Ring {
    lines: VecDeque::new(),
    pending: Vec::new(),
});

fn ring() -> MutexGuard<'static, Ring> {
    // A panic while holding the lock leaves a usable buffer behind.
    RING.lock().unwrap_or_else(PoisonError::into_inner)
}

fn cut(line: &str) -> String {
    match line.char_indices().nth(LINE_MAX) {
        Some((i, _)) => format!("{}…", &line[..i]),
        None => line.to_owned(),
    }
}

/// Keeps `line`; warnings and errors are also queued for MQTT.
pub fn push(line: &str, important: bool) {
    let line = cut(line);
    let mut r = ring();
    if r.lines.len() == LINES {
        r.lines.pop_front();
    }
    if important && r.pending.len() < PENDING_MAX {
        r.pending.push(line.clone());
    }
    r.lines.push_back(line);
}

/// All kept lines, oldest first, one per line.
#[must_use]
pub fn text() -> String {
    let lines: Vec<String> = ring().lines.iter().cloned().collect();
    let mut out = String::with_capacity(lines.iter().map(|l| l.len() + 1).sum());
    for line in &lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Warnings and errors to publish, and forgets them.
#[must_use]
pub fn take_pending() -> Vec<String> {
    std::mem::take(&mut ring().pending)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_last_lines_and_queues_warnings() {
        // One test, as the buffer is global.
        for i in 0..LINES + 5 {
            push(&format!("line {i}"), i % 40 == 0);
        }
        let text = text();
        assert!(!text.contains("line 4\n"), "oldest lines dropped");
        assert!(text.ends_with(&format!("line {}\n", LINES + 4)));
        assert_eq!(text.lines().count(), LINES);
        assert_eq!(take_pending(), ["line 0", "line 40", "line 80"]);
        assert!(take_pending().is_empty());
        push(&"x".repeat(500), false);
        assert!(
            super::text()
                .lines()
                .last()
                .is_some_and(|l| l.chars().count() == LINE_MAX + 1)
        );
    }
}
