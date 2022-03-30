use std::{cell::RefCell, thread::ThreadId};

use crate::ObserverTag;

thread_local! {
    /// `STORAGE` is a thread-localized instance of [`LocalStorage`].
    pub static STORAGE: RefCell<LocalStorage> = RefCell::new(LocalStorage::default());
}

/// `LocalStorage` is a thread-local structure that contains all necessary data for an [`Observer`].
///
/// In order to facilitate mutual reference, the thread-local data of all sub-modules
/// need to be stored centrally in `LocalStorage`.
#[derive(Default)]
pub struct LocalStorage {
    // The [`ObserverTag`] attached to the thread currently.
    attached_tag: ObserverTag,
}

pub struct LocalStorageRef {
    pub id: ThreadId,
    pub storage: LocalStorage,
}
