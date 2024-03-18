use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::{env, fs};

use closure::closure;

use pulse::callbacks::ListResult;
use pulse::context::introspect::{Introspector, SinkInfo};
use pulse::context::subscribe::{InterestMaskSet, Operation};
use pulse::context::{Context, ContextFlagSet, State};
use pulse::mainloop::standard::{IterateResult, Mainloop};
use pulse::proplist::Proplist;

use serde::Deserialize;

type FnSinkHandler = dyn FnMut(ListResult<&SinkInfo>);

#[derive(Deserialize)]
struct Config {
    sinks: Vec<String>,
}

fn main() {
    let sink_names = Arc::new(Mutex::new(HashSet::new()));
    let sink_indices = Arc::new(Mutex::new(HashSet::new()));
    load_config().map_or_else(
        || {
            eprintln!("no config file found: {}", get_config_file());
        },
        |config| {
            let mut set = sink_names.lock().unwrap();
            for sink in config.sinks {
                set.insert(sink);
            }
        },
    );
    start_volume_sync(sink_names, sink_indices);
}

fn get_config_file() -> String {
    let dir = match env::var("XDG_CONFIG_HOME") {
        Ok(v) => v,
        Err(_) => match env::var("HOME") {
            Ok(home) => format!("{home}/.config"),
            Err(_) => {
                eprintln!("failed to load $HOME var");
                ".".to_string()
            }
        },
    };
    format!("{dir}/volume-sync.toml")
}

fn load_config() -> Option<Config> {
    let filename = get_config_file();
    if let Ok(content) = fs::read_to_string(filename) {
        if let Ok(config) = toml::from_str(&content) {
            return Some(config);
        }
    }
    None
}

fn start_volume_sync(
    sink_names: Arc<Mutex<HashSet<String>>>,
    sink_indices: Arc<Mutex<HashSet<u32>>>,
) {
    let mut proplist = Proplist::new().unwrap();
    proplist
        .set_str(pulse::proplist::properties::APPLICATION_NAME, "volume-sync")
        .unwrap();
    let mut mainloop = Mainloop::new().expect("failed to create mainloop");
    let mut context = Context::new_with_proplist(&mainloop, "volume-sync", &proplist)
        .expect("failed to create context");

    context
        .connect(None, ContextFlagSet::NOFLAGS, None)
        .expect("failed to connect context");
    loop {
        match mainloop.iterate(false) {
            IterateResult::Quit(_) | IterateResult::Err(_) => {
                eprintln!("iterate result quit/err, quitting...");
                return;
            }
            IterateResult::Success(_) => {}
        }
        match context.get_state() {
            State::Ready => {
                println!("context ready");
                break;
            }
            State::Failed | State::Terminated => {
                eprintln!("context state failed/terminated, quitting...");
                return;
            }
            _ => {}
        }
    }

    let handler = make_sink_info_handler(sink_names.clone(), sink_indices.clone());
    context.introspect().get_sink_info_list(handler);

    let introspector = Rc::new(RefCell::new(context.introspect()));
    context.set_subscribe_callback(Some(Box::new(closure!(|_, op, index| {
        if let Some(o) = op {
            match o {
                Operation::New => {
                    println!("New({index})");
                    let handler = make_sink_info_handler(sink_names.clone(), sink_indices.clone());
                    introspector.borrow().get_sink_info_by_index(index, handler);
                }
                Operation::Changed => {
                    println!("Changed({index})");
                    let s = sink_indices.lock().unwrap();
                    if s.contains(&index) {
                        for k in s.iter() {
                            if *k != index {
                                println!("sync volume from:{index} to:{k}");
                                sync_volume(introspector.clone(), index, *k);
                            }
                        }
                    }
                }
                Operation::Removed => {
                    println!("Removed({index})");
                    let mut s = sink_indices.lock().unwrap();
                    if s.remove(&index) {
                        println!("dropped key");
                    }
                }
            }
        }
    }))));
    context.subscribe(InterestMaskSet::SINK, |success| {
        if !success {
            panic!("failed to subscribe context");
        }
    });

    mainloop.run().expect("failed to run mainloop");
}

fn sync_volume(introspector: Rc<RefCell<Introspector>>, from: u32, to: u32) {
    if from == to {
        return;
    }
    introspector.borrow_mut().get_sink_info_by_index(
        from,
        closure!(clone introspector, |result| if let ListResult::Item(sink_info) = result {
            if sink_info.index == from {
                introspector.borrow_mut().set_sink_volume_by_index(
                    to,
                    &sink_info.volume,
                    None,
                );
            }
        }),
    );
}

fn make_sink_info_handler(
    sink_names: Arc<Mutex<HashSet<String>>>,
    sink_indices: Arc<Mutex<HashSet<u32>>>,
) -> Box<FnSinkHandler> {
    Box::new(closure!(
        move sink_indices,
        |result| if let ListResult::Item(sink_info) = result {
            if let Some(name) = &sink_info.name {
                if sink_names.lock().unwrap().contains(name.as_ref()) {
                    sink_indices.lock().unwrap().insert(sink_info.index);
                }
            }
        }
    ))
}
