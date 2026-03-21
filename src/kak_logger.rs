use log::{Level, Log, Metadata, Record};
use std::process::{Command, Stdio};
use std::io::Write;

/// Logger that sends messages to Kakoune's *debug* buffer via `kak -p`
pub struct KakLogger {
    session: String,
    level: Level,
}

impl KakLogger {
    pub fn init(session: &str, level: Level) {
        let logger = Box::new(KakLogger {
            session: session.to_string(),
            level,
        });
        let _ = log::set_boxed_logger(logger);
        log::set_max_level(level.to_level_filter());
    }
}

impl Log for KakLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let msg = format!("kak-claude [{}]: {}", record.level(), record.args());
        // Write to *debug* buffer via kak -p (fire-and-forget)
        if let Ok(mut child) = Command::new("kak")
            .arg("-p")
            .arg(&self.session)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let escaped = msg.replace('\'', "''");
                let _ = write!(stdin, "echo -debug '{}'\n", escaped);
            }
        }
    }

    fn flush(&self) {}
}
