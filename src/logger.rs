use std::sync::Once;

use chrono::Local;
use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};

struct Logger;

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Info
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            println!(
                "[{}] {} - {}",
                Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.level(),
                record.args()
            );
        }
    }

    fn flush(&self) {}
}

static LOGGER: Logger = Logger;
static INIT: Once = Once::new();

pub fn init_logger() -> Result<(), SetLoggerError> {
    INIT.call_once(|| {
        log::set_logger(&LOGGER).unwrap();
        log::set_max_level(LevelFilter::Info);
    });
    Ok(())
}
