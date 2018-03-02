use std::env;
use log::{LogLevel, LogLevelFilter, LogRecord};
use env_logger::LogBuilder;

pub fn init() {
    let mut builder = LogBuilder::new();

    if env::var("RUST_LOG").is_ok() {
        // Load log info from environment variable
        builder.parse(&env::var("RUST_LOG").unwrap());
    } else {
        // Set default information
        let format = |record: &LogRecord| {
            if record.level() == LogLevel::Info {
                format!("{}", record.args())
            } else {
                format!(
                    "[{}:{}] {}",
                    record.location().module_path(),
                    record.level(),
                    record.args()
                )
            }
        };

        builder.format(format).filter(None, LogLevelFilter::Info);
        builder.parse("wifi-connect=info,iron::iron=off");
    }

    // Initialize the global logger instance. Panic on error.
    builder.init().unwrap();
}
