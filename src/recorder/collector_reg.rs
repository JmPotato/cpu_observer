// Copyright 2021 TiKV Project Authors. Licensed under Apache-2.0.

use crate::recorder::Task;
use crate::worker::Scheduler;
use crate::Collector;

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

/// `CollectorRegHandle` accepts registrations of [`Collector`].
///
/// [Collector]: crate::collector::Collector
#[derive(Clone)]
pub struct CollectorRegHandle {
    scheduler: Scheduler<Task>,
}

impl CollectorRegHandle {
    pub(crate) fn new(scheduler: Scheduler<Task>) -> Self {
        Self { scheduler }
    }

    /// Register a collector to the recorder. Dropping the returned [`CollectorGuard`] will
    /// deregister the corresponding collector.
    pub fn register(&self, collector: Box<dyn Collector>) -> CollectorGuard {
        static NEXT_COLLECTOR_ID: AtomicU64 = AtomicU64::new(1);
        let id = CollectorID(NEXT_COLLECTOR_ID.fetch_add(1, Ordering::SeqCst));

        let reg_msg = Task::CollectorReg(CollectorReg::Register { collector, id });
        match self.scheduler.schedule(reg_msg) {
            Ok(_) => CollectorGuard {
                id,
                tx: Some(self.scheduler.clone()),
            },
            Err(_err) => {
                // TODO: output the error message.
                CollectorGuard { id, tx: None }
            }
        }
    }
}

#[derive(Copy, Clone, Default, Debug, Eq, PartialEq, Hash)]
pub struct CollectorID(pub u64);

pub enum CollectorReg {
    Register {
        id: CollectorID,
        collector: Box<dyn Collector>,
    },
    Deregister {
        id: CollectorID,
    },
}

pub struct CollectorGuard {
    id: CollectorID,
    tx: Option<Scheduler<Task>>,
}

impl Drop for CollectorGuard {
    fn drop(&mut self) {
        if self.tx.is_none() {
            return;
        }

        if let Err(_err) = self
            .tx
            .as_ref()
            .unwrap()
            .schedule(Task::CollectorReg(CollectorReg::Deregister { id: self.id }))
        {
            // TODO: output the error message.
        }
    }
}
