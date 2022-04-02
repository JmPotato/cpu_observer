#![feature(core_intrinsics)]

mod collector;
mod local_storage;
mod record;
mod recorder;
mod worker;

use std::{intrinsics::unlikely, sync::Arc};

use local_storage::{LocalStorage, LocalStorageRef, STORAGE};

pub use crate::collector::Collector;
use crate::recorder::thread;
pub use crate::recorder::{
    init_recorder, CollectorGuard, CollectorRegHandle, Recorder, RecorderBuilder,
};
use crate::worker::Scheduler;

const MAX_THREAD_REGISTER_RETRY: u32 = 10;

/// ObserverTag is used to label the current thread or [`Future`] to attach different context info.
///
/// [`Future`]: std::future::Future
pub struct ObserverTag {
    infos: Arc<TagInfos>,
    observer_tag_factory: ObserverTagFactory,
}

impl ObserverTag {
    /// This method is used to provide [`ObserverTag`] with the ability
    /// to attach to the thread local context.
    ///
    /// When you call this method, the `ObserverTag` itself will be
    /// attached to [`STORAGE`], and a [`Guard`] used to control the life of the
    /// tag is returned. When the `Guard` is discarded, the tag (and other
    /// fields if necessary) in `STORAGE` will be cleaned up.
    ///
    /// [`STORAGE`]: crate::local_storage::STORAGE
    pub fn attach(&self) -> Guard {
        STORAGE.with(move |s| {
            let mut ls = s.borrow_mut();

            if unlikely(!ls.registered && ls.register_failed_times < MAX_THREAD_REGISTER_RETRY) {
                ls.registered = self.observer_tag_factory.register_local_storage(&ls);
                if !ls.registered {
                    ls.register_failed_times += 1;
                }
            }

            // unexpected nested attachment
            if ls.is_set {
                debug_assert!(false, "nested attachment is not allowed");
                return Guard;
            }

            let prev_tag = ls.attached_tag.swap(Some(self.infos.clone()));
            debug_assert!(prev_tag.is_none());
            ls.is_set = true;

            Guard
        })
    }
}

pub struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        STORAGE.with(|s| {
            let mut ls = s.borrow_mut();

            if !ls.is_set {
                return;
            }
            ls.is_set = false;

            // If the shared tag is occupied by the recorder thread
            // with `SharedTagInfos::load_full`, spin wait for releasing.
            loop {
                let tag_to_drop = ls.attached_tag.swap(None);
                if tag_to_drop.is_some() {
                    break;
                }
            }
        });
    }
}

#[derive(Clone)]
pub struct ObserverTagFactory {
    scheduler: Scheduler<recorder::Task>,
}

impl ObserverTagFactory {
    fn new(scheduler: Scheduler<recorder::Task>) -> Self {
        Self { scheduler }
    }

    pub fn new_tag(&self, name: impl Into<String>) -> ObserverTag {
        ObserverTag {
            infos: Arc::new(TagInfos { name: name.into() }),
            observer_tag_factory: self.clone(),
        }
    }

    fn register_local_storage(&self, storage: &LocalStorage) -> bool {
        let lsr = LocalStorageRef {
            id: thread::thread_id(),
            storage: storage.clone(),
        };
        match self.scheduler.schedule(recorder::Task::ThreadReg(lsr)) {
            Ok(_) => true,
            Err(_err) => false,
        }
    }
}

/// This structure is the actual internal data of [`ObserverTag`].
/// It represents the context of the `ObserverTag`.
#[derive(Debug, Default, Eq, PartialEq, Clone, Hash)]
pub struct TagInfos {
    // TODO: support more fields.
    name: String,
}

// TODO: add some test cases for this module.
#[cfg(test)]
mod tests {}
