use core::panic;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use closure::closure;
use pulse::callbacks::ListResult;
use pulse::context::introspect::{Introspector, SinkInfo};
use pulse::context::subscribe::{InterestMaskSet, Operation};
use pulse::context::{Context, ContextFlagSet, State};
use pulse::mainloop::standard::{IterateResult, Mainloop};
use pulse::proplist::Proplist;

type FnSinkHandler = dyn FnMut(ListResult<&SinkInfo>);

const CHAT_SINK_NAME: &str =
    "alsa_output.usb-Audeze_LLC_Audeze_Maxwell_Dongle_0000000000000000-01.pro-output-0";
const GAME_SINK_NAME: &str =
    "alsa_output.usb-Audeze_LLC_Audeze_Maxwell_Dongle_0000000000000000-01.pro-output-1";

fn main() {
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

    let sinks = Rc::new(RefCell::new(HashSet::new()));
    let handler = make_sink_info_handler(sinks.clone());
    context.introspect().get_sink_info_list(handler);

    let introspector = Rc::new(RefCell::new(context.introspect()));
    context.set_subscribe_callback(Some(Box::new(closure!(|_, op, index| {
        if let Some(o) = op {
            match o {
                Operation::New => {
                    println!("New({index})");
                    let handler = make_sink_info_handler(sinks.clone());
                    introspector.borrow().get_sink_info_by_index(index, handler);
                }
                Operation::Changed => {
                    println!("Changed({index})");
                    let s = sinks.borrow();
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
                    let mut s = sinks.borrow_mut();
                    if s.contains(&index) {
                        println!("dropping key");
                        s.remove(&index);
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

fn make_sink_info_handler(sinks: Rc<RefCell<HashSet<u32>>>) -> Box<FnSinkHandler> {
    Box::new(closure!(
        move sinks,
        |result| if let ListResult::Item(sink_info) = result {
            if let Some(name) = &sink_info.name {
                if name == GAME_SINK_NAME || name == CHAT_SINK_NAME {
                    sinks.borrow_mut().insert(sink_info.index);
                }
            }
        }
    ))
}
