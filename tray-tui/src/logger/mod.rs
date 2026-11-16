//! Logging that never draws over the TUI.
//!
//! Log lines go to stderr, but while the TUI owns the terminal ([`pause`]),
//! they are held back and printed after it is restored ([`resume`]).

use std::collections::VecDeque;
use std::io::{IsTerminal, Write};
use std::sync::Mutex;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

/// Held-back lines beyond this are dropped, oldest first.
const MAX_HELD: usize = 200;

/// `Some` while paused: the held-back output.
static HELD: Mutex<Option<VecDeque<Vec<u8>>>> = Mutex::new(None);

pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(PausableStderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
}

/// Hold log output back from now on.
pub fn pause() {
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    held.get_or_insert_with(VecDeque::new);
}

/// Print what was held back and write directly again.
pub fn resume() {
    let held = HELD.lock().unwrap_or_else(|e| e.into_inner()).take();
    let mut stderr = std::io::stderr().lock();
    for line in held.into_iter().flatten() {
        let _ = stderr.write_all(&line);
    }
}

struct PausableStderr;

impl<'a> MakeWriter<'a> for PausableStderr {
    type Writer = LineWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LineWriter(Vec::new())
    }
}

/// Collects one event, then hands it over on drop.
pub struct LineWriter(Vec<u8>);

impl Write for LineWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for LineWriter {
    fn drop(&mut self) {
        let line = std::mem::take(&mut self.0);
        let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
        match held.as_mut() {
            Some(queue) => {
                if queue.len() == MAX_HELD {
                    queue.pop_front();
                }
                queue.push_back(line);
            }
            None => {
                drop(held);
                let _ = std::io::stderr().write_all(&line);
            }
        }
    }
}

#[cfg(test)]
mod tests;
