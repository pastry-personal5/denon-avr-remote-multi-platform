//! What starts a shutdown, and what hurries one.
//!
//! A shutdown starts on SIGTERM, on SIGINT, or, in parent-pipe mode, when standard
//! input reaches its end. That last is how a process that started the server and
//! then died takes it down with it: the pipe the parent held closes. A signal that
//! arrives while a shutdown is already running means "now", and the process exits.

use tokio::io::AsyncReadExt;
use tokio::signal::unix::{signal, Signal, SignalKind};

pub const SIGINT: i32 = 2;
pub const SIGTERM: i32 = 15;

/// Why the server is stopping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Signal(i32),
    ParentGone,
}

/// The signals that stop the server, watched from the moment they are installed,
/// so none is lost between startup and the first wait.
pub struct Signals {
    terminate: Signal,
    interrupt: Signal,
}

impl Signals {
    pub fn install() -> std::io::Result<Self> {
        Ok(Self {
            terminate: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
        })
    }

    /// The number of the next SIGTERM or SIGINT.
    pub async fn next(&mut self) -> i32 {
        tokio::select! {
            _ = self.terminate.recv() => SIGTERM,
            _ = self.interrupt.recv() => SIGINT,
        }
    }
}

/// Resolves when standard input reaches its end or cannot be read. What the parent
/// writes before that is read and ignored.
pub async fn parent_gone() {
    let mut input = tokio::io::stdin();
    let mut buffer = [0u8; 256];
    loop {
        match input.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
    }
}
