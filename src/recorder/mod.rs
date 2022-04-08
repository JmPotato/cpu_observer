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
use crate::worker::Runnable;
use crate::worker::{Builder as WorkerBuilder, LazyWorker};
use crate::{Collector, ObserverTagFactory};

pub use self::collector_reg::{CollectorGuard, CollectorID, CollectorReg, CollectorRegHandle};
use self::sub_recorder::{CPURecorder, SubRecorder};
use self::thread::Pid;

// TODO: make the recording frequency configurable.
const RECORD_FREQUENCY: f64 = 99.0;
const RECORD_INTERVAL: Duration =
    Duration::from_micros((1_000.0 / RECORD_FREQUENCY * 1_000.0) as _);
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
    collectors: HashMap<CollectorID, Box<dyn Collector>>,
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
        self.sub_recorders.iter_mut().for_each(|sub_recorder| {
            sub_recorder.tick(&mut self.records);
        });
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
        if !self.running {
            return;
        }
        self.running = false;
        self.sub_recorders
            .iter_mut()
            .for_each(|sub_recorder| sub_recorder.pause());
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
        RECORD_INTERVAL
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
    name: impl Into<String>,
    collect_interval_ms: u64,
) -> (
    ObserverTagFactory,
    CollectorRegHandle,
    Box<LazyWorker<Task>>,
) {
    let recorder = RecorderBuilder::default()
        .collect_interval_ms(collect_interval_ms)
        .add_sub_recorder(Box::new(CPURecorder::default()))
        .build();
    let mut recorder_worker = WorkerBuilder::new(name.into())
        .pending_capacity(256)
        .create()
        .lazy_build();

    let observer_tag_factory = ObserverTagFactory::new(recorder_worker.scheduler());
    let collector_reg_handle = CollectorRegHandle::new(recorder_worker.scheduler());

    recorder_worker.start_with_timer(recorder);
    (
        observer_tag_factory,
        collector_reg_handle,
        Box::new(recorder_worker),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex,
        },
        thread::sleep,
    };

    use crate::TagInfos;

    use super::*;

    #[derive(Clone, Default)]
    struct MockSubRecorder {
        tick_count: Arc<AtomicUsize>,
        resume_count: Arc<AtomicUsize>,
        thread_created_count: Arc<AtomicUsize>,
        pause_count: Arc<AtomicUsize>,
    }

    impl SubRecorder for MockSubRecorder {
        fn tick(&mut self, records: &mut Records) {
            self.tick_count.fetch_add(1, Ordering::SeqCst);
            let tag = TagInfos::default();
            records.records.entry(Arc::new(tag)).or_default().cpu_time = 1;
        }

        fn pause(&mut self) {
            self.pause_count.fetch_add(1, Ordering::SeqCst);
        }

        fn resume(&mut self) {
            self.resume_count.fetch_add(1, Ordering::SeqCst);
        }

        fn on_thread_created(&mut self, _id: Pid, _store: &LocalStorage) {
            self.thread_created_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[derive(Clone, Default)]
    struct MockCollector {
        records: Arc<Mutex<Option<Arc<Records>>>>,
    }

    impl Collector for MockCollector {
        fn collect(&self, records: Arc<Records>) {
            *self.records.lock().unwrap() = Some(records);
        }
    }

    #[test]
    fn test_recorder_basic() {
        let sub_recorder = MockSubRecorder::default();
        let mut recorder = RecorderBuilder::default()
            .collect_interval_ms(20)
            .add_sub_recorder(Box::new(sub_recorder.clone()))
            .build();

        // register a new thread
        recorder.run(Task::ThreadReg(LocalStorageRef {
            id: 0,
            storage: LocalStorage::default(),
        }));
        recorder.on_timeout();
        assert!(!recorder.thread_stores.is_empty());
        assert_eq!(sub_recorder.pause_count.load(Ordering::SeqCst), 0);
        assert_eq!(sub_recorder.resume_count.load(Ordering::SeqCst), 0);
        assert_eq!(sub_recorder.tick_count.load(Ordering::SeqCst), 0);
        assert_eq!(sub_recorder.thread_created_count.load(Ordering::SeqCst), 1);

        // register a collector
        let collector = MockCollector::default();
        recorder.run(Task::CollectorReg(CollectorReg::Register {
            id: CollectorID(1),
            collector: Box::new(collector.clone()),
        }));
        recorder.on_timeout();
        assert_eq!(sub_recorder.pause_count.load(Ordering::SeqCst), 0);
        assert_eq!(sub_recorder.resume_count.load(Ordering::SeqCst), 1);
        assert_eq!(sub_recorder.tick_count.load(Ordering::SeqCst), 1);
        assert_eq!(sub_recorder.thread_created_count.load(Ordering::SeqCst), 1);

        // trigger collection
        sleep(Duration::from_millis(recorder.collect_interval_ms));
        recorder.on_timeout();
        assert_eq!(sub_recorder.pause_count.load(Ordering::SeqCst), 0);
        assert_eq!(sub_recorder.resume_count.load(Ordering::SeqCst), 1);
        assert_eq!(sub_recorder.tick_count.load(Ordering::SeqCst), 2);
        assert_eq!(sub_recorder.thread_created_count.load(Ordering::SeqCst), 1);
        let records = { collector.records.lock().unwrap().take().unwrap() };
        assert_eq!(records.records.len(), 1);
        assert_eq!(records.records.values().next().unwrap().cpu_time, 1);

        // deregister collector
        recorder.run(Task::CollectorReg(CollectorReg::Deregister {
            id: CollectorID(1),
        }));
        recorder.on_timeout();
        assert_eq!(sub_recorder.pause_count.load(Ordering::SeqCst), 1);
        assert_eq!(sub_recorder.resume_count.load(Ordering::SeqCst), 1);
        assert_eq!(sub_recorder.tick_count.load(Ordering::SeqCst), 2);
        assert_eq!(sub_recorder.thread_created_count.load(Ordering::SeqCst), 1);

        // nothing happens
        recorder.on_timeout();
        assert_eq!(sub_recorder.pause_count.load(Ordering::SeqCst), 1);
        assert_eq!(sub_recorder.resume_count.load(Ordering::SeqCst), 1);
        assert_eq!(sub_recorder.tick_count.load(Ordering::SeqCst), 2);
        assert_eq!(sub_recorder.thread_created_count.load(Ordering::SeqCst), 1);
    }
}
