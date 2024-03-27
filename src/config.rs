use std::{env, fs};

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub(crate) enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Config {
    pub(crate) sinks: Vec<String>,
    pub(crate) log_level: Option<LogLevel>,
}

impl Config {
    pub(crate) fn default() -> Config {
        return Config {
            sinks: Vec::new(),
            log_level: Some(LogLevel::Info),
        };
    }
}

impl LogLevel {
    pub(crate) fn to_level_filter(&self) -> log::LevelFilter {
        match self {
            LogLevel::Off => log::LevelFilter::Off,
            LogLevel::Error => log::LevelFilter::Error,
            LogLevel::Warn => log::LevelFilter::Warn,
            LogLevel::Info => log::LevelFilter::Info,
            LogLevel::Debug => log::LevelFilter::Debug,
            LogLevel::Trace => log::LevelFilter::Trace,
        }
    }
}

pub(crate) fn get_file() -> String {
    let dir = match env::var("XDG_CONFIG_HOME") {
        Ok(v) => v,
        Err(_) => match env::var("HOME") {
            Ok(home) => format!("{home}/.config"),
            Err(_) => {
                log::error!("failed to load $HOME var");
                ".".to_string()
            }
        },
    };
    format!("{dir}/volume-sync.toml")
}

pub(crate) fn load_config() -> Option<Config> {
    let filename = get_file();
    if let Ok(content) = fs::read_to_string(filename) {
        if let Ok(config) = toml::from_str(&content) {
            return Some(config);
        }
    }
    None
}
