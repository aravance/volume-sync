use std::cell::RefCell;
use std::collections::HashSet;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::{env, fs};

use closure::closure;

use pulse::callbacks::ListResult;
use pulse::context::subscribe::{InterestMaskSet, Operation};
use pulse::context::{Context, ContextFlagSet, State};
use pulse::mainloop::threaded::Mainloop;
use pulse::proplist::Proplist;

use serde::Deserialize;

#[derive(Deserialize)]
struct Config {
    sinks: Vec<String>,
}

#[derive(Debug)]
struct SinkDetails {
    index: u32,
    name: String,
}

#[derive(Debug)]
enum SinkEvent {
    New(SinkDetails),
    Changed(u32),
    Removed(u32),
}

fn main() {
    let sink_names = Arc::new(Mutex::new(HashSet::new()));
    let sink_indices = Arc::new(Mutex::new(HashSet::<u32>::new()));
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

    let mainloop = Rc::new(RefCell::new(
        Mainloop::new().expect("failed to create mainloop"),
    ));
    let (sender, receiver) = channel();
    let mut volume_sync = VolumeSync::new(mainloop.clone(), sender);

    println!("starting mainloop");
    mainloop.borrow_mut().lock();
    mainloop
        .borrow_mut()
        .start()
        .expect("failed to start mainloop");
    mainloop.borrow_mut().unlock();
    volume_sync.connect();

    let sinks = volume_sync.get_sinks();
    {
        let names = sink_names.lock().unwrap();
        let mut indices = sink_indices.lock().unwrap();
        for sink in sinks {
            if names.contains(&sink.name) {
                indices.insert(sink.index);
            }
        }
    }

    loop {
        println!("waiting for event");
        match receiver.recv() {
            Ok(e) => match &e {
                SinkEvent::New(sink) => {
                    let names = sink_names.lock().unwrap();
                    let mut indices = sink_indices.lock().unwrap();
                    if names.contains(&sink.name) {
                        indices.insert(sink.index);
                    }
                }
                SinkEvent::Changed(index) => {
                    let indices = sink_indices.lock().unwrap();
                    if indices.contains(&index) {
                        for i in indices.iter() {
                            volume_sync.sync_volume(*index, *i);
                        }
                    }
                }
                SinkEvent::Removed(index) => {
                    sink_indices.lock().unwrap().remove(index);
                }
            },
            Err(err) => eprintln!("error in receiver: {}", err),
        }
    }
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

struct VolumeSync {
    mainloop: Rc<RefCell<Mainloop>>,
    context: Rc<RefCell<Context>>,
    sender: Sender<SinkEvent>,
}

impl VolumeSync {
    fn sync_volume(&self, from: u32, to: u32) {
        if from == to {
            return;
        }

        println!("syncing volume from: {from} to: {to}");
        self.mainloop.borrow_mut().lock();
        self.context
            .borrow_mut()
            .introspect()
            .get_sink_info_by_index(
                from,
                closure!(
                    clone self.context,
                    |result| {
                        if let ListResult::Item(sink_info) = result {
                            context
                                .borrow_mut()
                                .introspect()
                                .set_sink_volume_by_index(to, &sink_info.volume, None);
                        }
                    }
                ),
            );
        self.mainloop.borrow_mut().unlock();
    }

    fn new(mainloop: Rc<RefCell<Mainloop>>, sender: Sender<SinkEvent>) -> VolumeSync {
        let mut proplist = Proplist::new().unwrap();
        proplist
            .set_str(pulse::proplist::properties::APPLICATION_NAME, "volume-sync")
            .unwrap();
        let context = Rc::new(RefCell::new(
            Context::new_with_proplist(mainloop.borrow().deref(), "volume-sync", &proplist)
                .expect("failed to create context"),
        ));

        println!("connecting context");
        context
            .borrow_mut()
            .connect(None, ContextFlagSet::NOFLAGS, None)
            .expect("failed to connect context");
        println!("setting state callback");
        context
            .borrow_mut()
            .set_state_callback(Some(Box::new(closure!(
                clone mainloop,
                clone context,
                || {
                    println!("got state callback");
                    let state = unsafe { (*context.as_ptr()).get_state() };
                    match state {
                        State::Ready | State::Failed | State::Terminated => unsafe {
                            (*mainloop.as_ptr()).signal(false);
                        },
                        _ => {}
                    }
                }
            ))));

        return VolumeSync {
            mainloop,
            context,
            sender,
        };
    }

    fn get_sinks(&self) -> Vec<SinkDetails> {
        let out = Arc::new(Mutex::new(Some(Vec::new())));
        self.mainloop.borrow_mut().lock();
        println!("get_sink_info_list");
        let op = self
            .context
            .borrow_mut()
            .introspect()
            .get_sink_info_list(closure!(
                clone self.mainloop,
                clone out,
                |result| {
                    println!("result: {result:?}");
                    if let ListResult::Item(sink_info) = result {
                        if let Some(o) = &mut *out.lock().unwrap() {
                            let name = sink_info.name.as_ref().map_or_else(
                                || "".to_string(),
                                |it| it.to_string(),
                            );
                            o.push(SinkDetails {
                                index: sink_info.index,
                                name: name.to_string(),
                            });
                        }
                    }
                    unsafe { (*mainloop.as_ptr()).signal(false); }
                }
            ));
        println!("watch for state");
        loop {
            match op.get_state() {
                pulse::operation::State::Running => self.mainloop.borrow_mut().wait(),
                pulse::operation::State::Done => break,
                pulse::operation::State::Cancelled => break,
            }
        }
        self.mainloop.borrow_mut().unlock();
        return out.lock().unwrap().take().unwrap();
    }

    fn connect(&mut self) {
        self.mainloop.borrow_mut().lock();

        loop {
            match self.context.borrow().get_state() {
                State::Ready => {
                    break;
                }
                State::Failed | State::Terminated => {
                    eprintln!("context state failed/terminated, quitting...");
                    self.mainloop.borrow_mut().unlock();
                    self.mainloop.borrow_mut().stop();
                    return;
                }
                _ => {
                    self.mainloop.borrow_mut().wait();
                }
            }
        }
        self.context.borrow_mut().set_state_callback(None);

        println!("setting subscribe callback");
        self.context.borrow_mut().set_subscribe_callback(Some(Box::new(closure!(
            clone self.sender,
            clone self.context,
            |_, op, index| {
                println!("got subscribe callback");
                if let Some(o) = op {
                    match o {
                        Operation::New => {
                            println!("New({index})");
                            context.borrow_mut().introspect().get_sink_info_by_index(index, closure!(
                                clone sender,
                                move index,
                                |result| {
                                    if let ListResult::Item(sink_info) = result {
                                        if let Some(name) = &sink_info.name {
                                            sender.send(SinkEvent::New(SinkDetails{
                                                name: name.to_string(),
                                                index: index,
                                            })).expect("failed to send");
                                        }
                                    }
                                }));
                        }
                        Operation::Changed => {
                            println!("Changed({index})");
                            sender.send(SinkEvent::Changed(index)).expect("failed to send new event");
                        }
                        Operation::Removed => {
                            println!("Removed({index})");
                            sender.send(SinkEvent::Removed(index)).expect("failed to send new event");
                        }
                    }
                }
            }
        ))));

        println!("subscribing context");
        self.context
            .borrow_mut()
            .subscribe(InterestMaskSet::SINK, |success| {
                println!("got subscribe context");
                if !success {
                    panic!("failed to subscribe context");
                }
            });

        self.mainloop.borrow_mut().unlock();
    }
}
