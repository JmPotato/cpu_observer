use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::TagInfos;

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Records {
    pub begin_unix_time_secs: u64,
    pub duration: Duration,

    // tag -> record
    pub records: HashMap<Arc<TagInfos>, Record>,
}

#[derive(Debug, Default, Copy, Clone, Eq, PartialEq)]
pub struct Record {
    pub cpu_time: u32, // ms
}
