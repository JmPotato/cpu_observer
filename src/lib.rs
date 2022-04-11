#![feature(core_intrinsics)]

mod collector;
mod local_storage;
mod record;
mod recorder;
mod util;
mod worker;

use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::{intrinsics::unlikely, sync::Arc};

pub use crate::collector::Collector;
use crate::local_storage::{LocalStorage, LocalStorageRef, STORAGE};
pub use crate::record::Records;
pub use crate::recorder::{
    init_recorder, CollectorGuard, CollectorRegHandle, Recorder, RecorderBuilder,
};
use crate::recorder::{thread, Task};
use crate::worker::{LazyWorker, Scheduler};

// This is used to export the internal worker type.
pub type RecorderWorker = LazyWorker<Task>;

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

    pub fn update(&self, name: impl Into<String>) {
        *self.infos.name.lock().unwrap() = Some(name.into());
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

    #[allow(dead_code)]
    fn new_for_test() -> Self {
        use crate::worker::Builder as WorkerBuilder;
        Self::new(
            WorkerBuilder::new("test_worker")
                .pending_capacity(256)
                .create()
                .lazy_build()
                .scheduler(),
        )
    }

    pub fn new_tag(&self, name: impl Into<String>) -> ObserverTag {
        ObserverTag {
            infos: Arc::new(TagInfos {
                name: Mutex::new(Some(name.into())),
            }),
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
#[derive(Default, Debug)]
pub struct TagInfos {
    // TODO: do we really need a Mutex here?
    name: Mutex<Option<String>>,
    // TODO: support more fields.
}

impl PartialEq for TagInfos {
    fn eq(&self, other: &Self) -> bool {
        *self.name.lock().unwrap() == *other.name.lock().unwrap()
    }
}

impl Eq for TagInfos {}

impl Hash for TagInfos {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.lock().unwrap().hash(state);
    }
}

// TODO: add some test cases for this module.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attach() {
        // Use a thread created by ourself. If we use unit test thread directly,
        // the test results may be affected by parallel testing.
        std::thread::spawn(|| {
            let observer_tag_factory = ObserverTagFactory::new_for_test();
            let tag = ObserverTag {
                infos: Arc::new(TagInfos::default()),
                observer_tag_factory,
            };
            {
                let _guard = tag.attach();
                STORAGE.with(|s| {
                    let ls = s.borrow_mut();
                    let local_tag = ls.attached_tag.swap(None);
                    assert!(local_tag.is_some());
                    let tag_infos = local_tag.unwrap();
                    assert_eq!(tag_infos, tag.infos);
                    assert!(ls.attached_tag.swap(Some(tag_infos)).is_none());
                });
                // drop here.
            }
            STORAGE.with(|s| {
                let ls = s.borrow_mut();
                let local_tag = ls.attached_tag.swap(None);
                assert!(local_tag.is_none());
            });
        })
        .join()
        .unwrap();
    }
}
