use chrono::Local;
use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};
use std::sync::OnceLock;
struct Logger {
    version: String,
}

impl Logger {
    fn new(version: &str) -> Self {
        Logger {
            version: version.to_string(),
        }
    }
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Info
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            println!(
                "[{}] [{}] {} - {}",
                Local::now().format("%Y-%m-%d %H:%M:%S"),
                self.version, 
                record.level(),
                record.args()
            );
        }
    }

    fn flush(&self) {}
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

pub fn init_logger(version: &str) -> Result<(), SetLoggerError> {
    let logger = Logger::new(version);
    LOGGER.set(logger).ok(); 
    log::set_logger(LOGGER.get().unwrap())?;
    log::set_max_level(LevelFilter::Info);
    Ok(())
}