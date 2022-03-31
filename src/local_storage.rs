use std::cell::RefCell;
use std::sync::Arc;

use crossbeam::atomic::AtomicCell;

use crate::recorder::thread::Pid;
use crate::TagInfos;

thread_local! {
    /// `STORAGE` is a thread-localized instance of [`LocalStorage`].
    pub static STORAGE: RefCell<LocalStorage> = RefCell::new(LocalStorage::default());
}

#[derive(Clone, Default)]
pub struct SharedTagInfos {
    tag: Arc<AtomicCell<Option<Arc<TagInfos>>>>,
}

impl SharedTagInfos {
    // Only used for test.
    #![allow(dead_code)]
    pub(crate) fn new(tag: Arc<TagInfos>) -> Self {
        Self {
            tag: Arc::new(AtomicCell::new(Some(tag))),
        }
    }

    pub fn swap(&self, tag: Option<Arc<TagInfos>>) -> Option<Arc<TagInfos>> {
        self.tag.swap(tag)
    }

    pub fn load_full(&self) -> Option<Arc<TagInfos>> {
        self.tag.swap(None).map(|t| {
            let r = t.clone();
            assert!(self.tag.swap(Some(t)).is_none());
            r
        })
    }
}

/// `LocalStorage` is a thread-local structure that contains all necessary data for an [`Observer`].
///
/// In order to facilitate mutual reference, the thread-local data of all sub-modules
/// need to be stored centrally in `LocalStorage`.
#[derive(Clone, Default)]
pub struct LocalStorage {
    pub registered: bool,
    pub register_failed_times: u32,
    pub is_set: bool,
    // The [`ObserverTag`] attached to the thread currently.
    pub(crate) attached_tag: SharedTagInfos,
}

pub struct LocalStorageRef {
    pub id: Pid,
    pub storage: LocalStorage,
}
