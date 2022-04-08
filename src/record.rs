use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::util::now_unix_time;
use crate::TagInfos;

#[derive(Eq, PartialEq, Clone, Debug)]
pub struct Records {
    pub begin_unix_time_secs: u64,
    pub duration: Duration,

    // tag -> record
    pub records: HashMap<Arc<TagInfos>, Record>,
}

impl Default for Records {
    fn default() -> Self {
        Self {
            begin_unix_time_secs: now_unix_time().as_secs(),
            duration: Duration::default(),
            records: HashMap::default(),
        }
    }
}

#[derive(Default, Debug, Copy, Clone, Eq, PartialEq)]
pub struct Record {
    pub cpu_time: u32, // ms
}
