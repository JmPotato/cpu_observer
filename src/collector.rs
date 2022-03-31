use std::sync::Arc;

use crate::record::Records;

pub trait Collector: Send {
    fn collect(&self, records: Arc<Records>);
}
