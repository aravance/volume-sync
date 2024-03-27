use std::cell::RefCell;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use pulse::callbacks::ListResult;
use pulse::context::subscribe::{InterestMaskSet, Operation};
use pulse::context::{Context, ContextFlagSet, State};
use pulse::mainloop::threaded::Mainloop;
use pulse::proplist::Proplist;

use closure::closure;

#[derive(Debug)]
pub(crate) struct SinkDetails {
    pub(crate) index: u32,
    pub(crate) name: String,
}

#[derive(Debug)]
pub(crate) enum VolumeSyncEvent {
    SinkNew(SinkDetails),
    SinkChanged(u32),
    SinkRemoved(u32),
    ConfigChanged,
}

pub(crate) struct VolumeSync {
    pub(crate) mainloop: Rc<RefCell<Mainloop>>,
    pub(crate) context: Rc<RefCell<Context>>,
    pub(crate) sender: Sender<VolumeSyncEvent>,
}

impl VolumeSync {
    pub(crate) fn sync_volume(&self, from: u32, to: u32) {
        if from == to {
            return;
        }

        log::info!("syncing volume: {from} -> {to}");
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

    pub(crate) fn new(
        mainloop: Rc<RefCell<Mainloop>>,
        sender: Sender<VolumeSyncEvent>,
    ) -> VolumeSync {
        let mut proplist = Proplist::new().unwrap();
        proplist
            .set_str(pulse::proplist::properties::APPLICATION_NAME, "volume-sync")
            .unwrap();
        let context = Rc::new(RefCell::new(
            Context::new_with_proplist(mainloop.borrow().deref(), "volume-sync", &proplist)
                .expect("failed to create context"),
        ));

        log::info!("connecting context");
        context
            .borrow_mut()
            .connect(None, ContextFlagSet::NOFLAGS, None)
            .expect("failed to connect context");

        return VolumeSync {
            mainloop,
            context,
            sender,
        };
    }

    pub(crate) fn connect(&mut self) -> Result<(), &'static str> {
        self.mainloop.borrow_mut().lock();

        log::debug!("setting state callback");
        self.context
            .borrow_mut()
            .set_state_callback(Some(Box::new(closure!(
                clone self.mainloop,
                clone self.context,
                || {
                    log::debug!("got state callback");
                    let state = unsafe { (*context.as_ptr()).get_state() };
                    match state {
                        State::Ready | State::Failed | State::Terminated => {
                            unsafe { (*mainloop.as_ptr()).signal(false); }
                        },
                        _ => {},
                    }
                }
            ))));

        loop {
            match self.context.borrow().get_state() {
                State::Ready => {
                    break;
                }
                State::Failed | State::Terminated => {
                    log::error!("context state failed/terminated, quitting...");
                    self.mainloop.borrow_mut().unlock();
                    self.mainloop.borrow_mut().stop();
                    return Err("failed to get ready context");
                }
                _ => {
                    self.mainloop.borrow_mut().wait();
                }
            }
        }
        self.context.borrow_mut().set_state_callback(None);

        log::debug!("setting subscribe callback");
        self.context.borrow_mut().set_subscribe_callback(Some(Box::new(closure!(
            clone self.sender,
            clone self.context,
            |_, op, index| {
                log::debug!("got subscribe callback");
                if let Some(o) = op {
                    match o {
                        Operation::New => {
                            log::info!("New({index})");
                            context
                                .borrow_mut()
                                .introspect()
                                .get_sink_info_by_index(index, closure!(
                                    clone sender,
                                    move index,
                                    |result| {
                                        if let ListResult::Item(sink_info) = result {
                                            if let Some(name) = &sink_info.name {
                                                sender
                                                    .send(VolumeSyncEvent::SinkNew(SinkDetails{
                                                        name: name.to_string(),
                                                        index: index,
                                                    }))
                                                    .expect("failed to send");
                                            }
                                        }
                                    }
                                ));
                        }
                        Operation::Changed => {
                            log::info!("Changed({index})");
                            sender.send(VolumeSyncEvent::SinkChanged(index)).expect("failed to send new event");
                        }
                        Operation::Removed => {
                            log::info!("Removed({index})");
                            sender.send(VolumeSyncEvent::SinkRemoved(index)).expect("failed to send new event");
                        }
                    }
                }
            }
        ))));

        log::info!("subscribing to sink events");
        self.context
            .borrow_mut()
            .subscribe(InterestMaskSet::SINK, |success| {
                log::debug!("got subscribe context");
                if !success {
                    panic!("failed to subscribe context");
                }
            });

        self.mainloop.borrow_mut().unlock();

        Ok(())
    }

    pub(crate) fn get_sinks(&self) -> Vec<SinkDetails> {
        let out = Arc::new(Mutex::new(Some(Vec::new())));
        self.mainloop.borrow_mut().lock();
        log::debug!("get_sink_info_list");
        let op = self
            .context
            .borrow_mut()
            .introspect()
            .get_sink_info_list(closure!(
                clone self.mainloop,
                clone out,
                |result| {
                    log::debug!("result: {result:?}");
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
        log::debug!("watch for state");
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
}
