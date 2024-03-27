use std::cell::RefCell;
use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

use closure::closure;

use notify::event::{ModifyKind, RemoveKind, RenameMode};
use notify::{EventKind, RecursiveMode, Watcher};

use pulse::mainloop::threaded::Mainloop;

use simple_logger::SimpleLogger;

mod config;
use crate::config::{Config, LogLevel};

mod volume_sync;
use crate::volume_sync::{VolumeSync, VolumeSyncEvent};

fn main() {
    SimpleLogger::new().init().unwrap();
    log::set_max_level(log::LevelFilter::Info);

    let sink_indices = Arc::new(Mutex::new(HashSet::new()));
    let config = Arc::new(Mutex::new(Config::default()));
    let (sender, receiver) = channel();

    let handle_config_change = |c: Config| {
        let log_level = c.log_level.unwrap_or(LogLevel::Info);
        *config.lock().unwrap() = Config {
            log_level: Some(log_level.clone()),
            sinks: c.sinks.clone(),
        };
        log::debug!("new config: {:?}", config.lock().unwrap());
        log::info!("set log_level to {log_level:?}");
        log::set_max_level(log_level.to_level_filter());
    };

    if let Some(c) = config::load_config() {
        handle_config_change(c);
    } else {
        log::warn!("no config file found: {}", config::get_file());
        handle_config_change(Config::default());
    };

    let mut watcher = notify::recommended_watcher(closure!(
        clone sender,
        |res: notify::Result<notify::Event>| match res {
            Ok(event) => {
                if event.paths.first().map_or("", |p| p.to_str().expect("failed to get event path")) == config::get_file() {
                    match event.kind {
                        EventKind::Modify(ModifyKind::Name(RenameMode::To))
                        | EventKind::Modify(ModifyKind::Name(RenameMode::From))
                        | EventKind::Modify(ModifyKind::Data(_))
                        | EventKind::Remove(RemoveKind::File) => {
                            log::info!("event: {event:?}");
                            sender.send(VolumeSyncEvent::ConfigChanged).expect("failed to send config event");
                        }
                        _ => log::debug!("ignore event: {event:?}"),
                    }
                }
            }
            Err(e) => log::error!("error: {e:?}"),
        }
    )).expect("failed to create config file watcher");
    log::info!("starting config file watcher");
    watcher
        .watch(
            Path::new(&config::get_file())
                .parent()
                .expect("no parent dir"),
            RecursiveMode::NonRecursive,
        )
        .expect("failed to start config file watcher");

    let mainloop = Rc::new(RefCell::new(
        Mainloop::new().expect("failed to create mainloop"),
    ));
    let volume_sync = Rc::new(RefCell::new(VolumeSync::new(mainloop.clone(), sender)));

    log::info!("starting mainloop");
    mainloop.borrow_mut().lock();
    mainloop
        .borrow_mut()
        .start()
        .expect("failed to start mainloop");
    mainloop.borrow_mut().unlock();
    volume_sync
        .borrow_mut()
        .connect()
        .expect("failed to connect volume_sync");

    let update_sink_indices = || {
        let sinks = volume_sync.borrow().get_sinks();
        let cfg = config.lock().unwrap();
        let mut indices = sink_indices.lock().unwrap();
        indices.clear();
        for sink in sinks {
            if cfg.sinks.contains(&sink.name) {
                indices.insert(sink.index);
            }
        }
    };
    update_sink_indices();

    loop {
        log::debug!("waiting for event");
        match receiver.recv() {
            Ok(e) => match &e {
                VolumeSyncEvent::SinkNew(sink) => {
                    let names = &config.lock().unwrap().sinks;
                    let mut indices = sink_indices.lock().unwrap();
                    if names.contains(&sink.name) {
                        indices.insert(sink.index);
                    }
                }
                VolumeSyncEvent::SinkChanged(index) => {
                    let indices = sink_indices.lock().unwrap();
                    if indices.contains(&index) {
                        for i in indices.iter() {
                            volume_sync.borrow().sync_volume(*index, *i);
                        }
                    }
                }
                VolumeSyncEvent::SinkRemoved(index) => {
                    sink_indices.lock().unwrap().remove(index);
                }
                VolumeSyncEvent::ConfigChanged => {
                    if let Some(c) = config::load_config() {
                        handle_config_change(c);
                    } else {
                        log::warn!("no config file found: {}", config::get_file());
                        handle_config_change(Config::default());
                    };
                    log::debug!("fetch sinks");
                    update_sink_indices();
                }
            },
            Err(err) => log::warn!("error in receiver: {}", err),
        }
    }
}
