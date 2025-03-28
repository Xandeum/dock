use std::sync::Once;
use chrono::Local;
use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};

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

static mut LOGGER: Option<Logger> = None; 
static INIT: Once = Once::new();

pub fn init_logger(version: &str) -> Result<(), SetLoggerError> {
    INIT.call_once(|| {
        unsafe {
            LOGGER = Some(Logger::new(version));
            log::set_logger(LOGGER.as_ref().unwrap()).unwrap();
            log::set_max_level(LevelFilter::Info);
        }
    });
    Ok(())
}