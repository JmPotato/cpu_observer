mod local_storage;
mod record;
mod recorder;
mod worker;

use std::sync::Arc;

use local_storage::STORAGE;

/// ObserverTag is used to label the current thread or [`Future`] to attach different context info.
///
/// [`Future`]: std::future::Future
#[derive(Default)]
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
        STORAGE.with(|_s| Guard {})
    }
}

pub struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {}
}

#[derive(Default)]
pub struct ObserverTagFactory {}

/// This structure is the actual internal data of [`ObserverTag`].
/// It represents the context of the `ObserverTag`.
#[derive(Debug, Default, Eq, PartialEq, Clone, Hash)]
pub struct TagInfos {
    // TODO: support more fields.
    name: String,
}

// TODO: add some test cases for this module.
#[cfg(test)]
mod tests {
    use super::*;
}
