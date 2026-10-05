//! Logging ([§3.4]): `<state>/logs/mlo.log`, rotating, plus a per-run journal.

use crate::config::Config;
use tracing_subscriber::prelude::*;

/// Initialise tracing to `<music>/.mlo/logs/mlo.log` (rotating daily) and,
/// when `stderr` is true, mirror to stderr. Returns a guard that must be kept
/// alive for the process lifetime.
pub fn init(cfg: &Config, stderr: bool) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let dir = cfg.logs_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let appender = tracing_appender::rolling::daily(&dir, "mlo.log");
    let (nb, guard) = tracing_appender::non_blocking(appender);

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(nb)
        .with_ansi(false)
        .with_target(false);

    let registry = tracing_subscriber::registry().with(file_layer);

    if stderr && std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        let stderr_layer = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(false);
        let _ = registry.with(stderr_layer).try_init();
    } else {
        let _ = registry.try_init();
    }
    Some(guard)
}