mod collector_reg;
mod sub_recorder;
pub mod thread;

use std::collections::{HashMap, HashSet};
use std::fmt::{self, Display};
use std::mem;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::local_storage::{LocalStorage, LocalStorageRef};
use crate::record::Records;
use crate::worker::Builder as WorkerBuilder;
use crate::worker::Runnable;
use crate::{Collector, ObserverTagFactory};

use self::collector_reg::{CollectorId, CollectorReg, CollectorRegHandle};
use self::sub_recorder::{CPURecorder, SubRecorder};
use self::thread::Pid;

const RECORD_LEN_THRESHOLD: usize = 20_000;
const CLEANUP_INTERVAL_SECS: u64 = 15 * 60;

pub struct Recorder {
    // Recorder will make every collector to collect the current recorded results every collect_interval.
    collect_interval_ms: u64,
    last_collect: Instant,
    cleanup_interval_second: u64,
    last_cleanup: Instant,
    running: bool,

    records: Records,
    thread_stores: HashMap<Pid, LocalStorage>,
    sub_recorders: Vec<Box<dyn SubRecorder>>,
    collectors: HashMap<CollectorId, Box<dyn Collector>>,
}

impl Recorder {
    fn handle_collector_registration(&mut self, reg: CollectorReg) {
        match reg {
            CollectorReg::Register { id, collector } => {
                self.collectors.insert(id, collector);
            }
            CollectorReg::Deregister { id } => {
                self.collectors.remove(&id);
            }
        }
    }

    fn handle_thread_registration(&mut self, lsr: LocalStorageRef) {
        self.thread_stores.insert(lsr.id, lsr.storage.clone());
        self.sub_recorders
            .iter_mut()
            .for_each(|sub_recorder| sub_recorder.on_thread_created(lsr.id, &lsr.storage));
    }

    fn tick(&mut self) {
        // Tick every sub-recorder to handle the records.
        self.sub_recorders
            .iter_mut()
            .for_each(|sub_recorder| sub_recorder.tick(&mut self.records));
        // Check if we should call a collector to collect the records.
        let collect_interval = Instant::now().saturating_duration_since(self.last_collect);
        if collect_interval.as_millis() > self.collect_interval_ms as _ {
            let mut records = mem::take(&mut self.records);
            records.duration = collect_interval;
            if !records.records.is_empty() {
                let records = Arc::new(records);
                self.collectors
                    .values()
                    .for_each(|collector| collector.collect(records.clone()));
            }
            self.last_collect = Instant::now();
        }
    }

    fn clean_up(&mut self) {
        if Instant::now()
            .saturating_duration_since(self.last_cleanup)
            .as_secs()
            > self.cleanup_interval_second
        {
            // Clean up the data of the destroyed threads.
            if let Ok(ids) = thread::thread_ids::<HashSet<_>>(thread::process_id()) {
                self.thread_stores.retain(|k, v| {
                    let retain = ids.contains(&(*k as _));
                    debug_assert!(retain || v.attached_tag.swap(None).is_none());
                    retain
                });
            }
            // Shrink the memory usage of the records.
            if self.records.records.capacity() > RECORD_LEN_THRESHOLD
                && self.records.records.len() < (RECORD_LEN_THRESHOLD / 2)
            {
                self.records.records.shrink_to(RECORD_LEN_THRESHOLD);
            }
            // Call every sub-recorder to clean up.
            self.sub_recorders
                .iter_mut()
                .for_each(|sub_recorder| sub_recorder.clean_up());
            self.last_cleanup = Instant::now();
        }
    }

    fn pause(&mut self) {
        self.running = false;
    }

    fn resume(&mut self) {
        if self.running {
            return;
        }
        self.running = true;

        let now = Instant::now();
        self.records = Records::default();
        self.last_collect = now;
        self.last_cleanup = now;
        self.sub_recorders
            .iter_mut()
            .for_each(|sub_recorder| sub_recorder.resume());
    }
}

pub enum Task {
    CollectorReg(CollectorReg),
    ThreadReg(LocalStorageRef),
}

impl Display for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Task::CollectorReg(_) => {
                write!(f, "CollectorReg")?;
            }
            Task::ThreadReg(_) => {
                write!(f, "NewThread")?;
            }
        }
        Ok(())
    }
}

impl Runnable for Recorder {
    type Task = Task;

    fn run(&mut self, task: Task) {
        match task {
            Task::CollectorReg(reg) => self.handle_collector_registration(reg),
            Task::ThreadReg(lsr) => self.handle_thread_registration(lsr),
        }
    }

    fn on_timeout(&mut self) {
        if self.collectors.is_empty() {
            self.pause();
            return;
        } else {
            self.resume();
        }

        self.tick();
        self.clean_up();
    }

    fn get_interval(&self) -> Duration {
        Duration::from_millis(0)
    }

    fn shutdown(&mut self) {
        unimplemented!()
    }
}

pub struct RecorderBuilder {
    collect_interval_ms: u64,
    cleanup_interval_second: u64,
    sub_recorders: Vec<Box<dyn SubRecorder>>,
}

impl Default for RecorderBuilder {
    fn default() -> Self {
        Self {
            collect_interval_ms: 1000,
            cleanup_interval_second: CLEANUP_INTERVAL_SECS,
            sub_recorders: Vec::new(),
        }
    }
}

impl RecorderBuilder {
    #[must_use]
    pub fn collect_interval_ms(mut self, collect_interval_ms: u64) -> Self {
        self.collect_interval_ms = collect_interval_ms;
        self
    }

    pub fn cleanup_interval_second(mut self, cleanup_interval_second: u64) -> Self {
        self.cleanup_interval_second = cleanup_interval_second;
        self
    }

    #[must_use]
    pub fn add_sub_recorder(mut self, r: Box<dyn SubRecorder>) -> Self {
        self.sub_recorders.push(r);
        self
    }

    pub fn build(self) -> Recorder {
        let now = Instant::now();
        Recorder {
            collect_interval_ms: self.collect_interval_ms,
            last_collect: now,
            cleanup_interval_second: self.cleanup_interval_second,
            last_cleanup: now,
            running: false,
            records: Records::default(),
            thread_stores: HashMap::default(),
            sub_recorders: self.sub_recorders,
            collectors: HashMap::default(),
        }
    }
}

pub fn init_recorder(
    collect_interval_ms: u64,
    cleanup_interval_second: u64,
) -> (ObserverTagFactory, CollectorRegHandle) {
    let recorder = RecorderBuilder::default()
        .collect_interval_ms(collect_interval_ms)
        .cleanup_interval_second(cleanup_interval_second)
        .add_sub_recorder(Box::new(CPURecorder::default()))
        .build();
    let mut recorder_worker = WorkerBuilder::new("cpu-observer-recorder")
        .pending_capacity(256)
        .create()
        .lazy_build();

    let observer_tag_factory = ObserverTagFactory::new(recorder_worker.scheduler());
    let collector_reg_handle = CollectorRegHandle::new(recorder_worker.scheduler());

    recorder_worker.start_with_timer(recorder);
    (observer_tag_factory, collector_reg_handle)
}
