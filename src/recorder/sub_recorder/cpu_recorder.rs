use std::collections::HashMap;

use crate::{
    local_storage::{LocalStorage, SharedTagInfos},
    record::{Record, Records},
    recorder::thread::{self, Pid},
};

use super::SubRecorder;

/// An implementation of [`SubRecorder`] for collecting cpu statistics.
///
/// `CPURecorder` collects cpu usage at a fixed frequency.
///
/// [SubRecorder]: crate::recorder::SubRecorder
#[derive(Default)]
pub struct CPURecorder {
    thread_stats: HashMap<Pid, ThreadStat>,
}

impl SubRecorder for CPURecorder {
    fn tick(&mut self, records: &mut Records) {
        #[allow(clippy::mutable_key_type)]
        let records = &mut records.records;
        let pid = thread::process_id();
        self.thread_stats.iter_mut().for_each(|(tid, thread_stat)| {
            // TODO: more precise CPU usage calculation when covering multiply tags.
            let cur_tag = thread_stat.attached_tag.load_full();
            if let Some(cur_tag) = cur_tag {
                if let Ok(cur_stat) = thread::thread_stat(pid, *tid) {
                    let last_stat = &thread_stat.stat;
                    if *last_stat != cur_stat {
                        let delta_ms =
                            (cur_stat.total_cpu_time() - last_stat.total_cpu_time()) * 1_000.;
                        let record = records.entry(cur_tag).or_insert_with(Record::default);
                        record.cpu_time += delta_ms as u32;
                    }
                    thread_stat.stat = cur_stat;
                }
            }
        });
    }

    fn clean_up(&mut self) {
        const THREAD_STAT_LEN_THRESHOLD: usize = 500;

        if self.thread_stats.capacity() > THREAD_STAT_LEN_THRESHOLD
            && self.thread_stats.len() < THREAD_STAT_LEN_THRESHOLD / 2
        {
            self.thread_stats.shrink_to(THREAD_STAT_LEN_THRESHOLD);
        }
    }

    fn resume(&mut self) {
        let pid = thread::process_id();
        for (tid, stat) in &mut self.thread_stats {
            stat.stat = thread::thread_stat(pid, *tid).unwrap_or_default();
        }
    }

    fn on_thread_created(&mut self, id: Pid, store: &LocalStorage) {
        self.thread_stats.insert(
            id,
            ThreadStat {
                attached_tag: store.attached_tag.clone(),
                stat: Default::default(),
            },
        );
    }
}

struct ThreadStat {
    attached_tag: SharedTagInfos,
    stat: thread::ThreadStat,
}

#[cfg(test)]
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn test_record() {
        let mut recorder = CpuRecorder::default();
        let mut records = RawRecords::default();
        recorder.tick(&mut records, &mut HashMap::default());
        assert!(records.records.is_empty());
    }
}

#[cfg(test)]
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod tests {
    use crate::TagInfos;

    use super::*;
    use std::sync::Arc;

    fn heavy_job() -> u64 {
        let m: u64 = rand::random();
        let n: u64 = rand::random();
        let m = m ^ n;
        let n = m.wrapping_mul(n);
        let m = m.wrapping_add(n);
        let n = m & n;
        let m = m | n;
        m.wrapping_sub(n)
    }

    #[test]
    fn test_record() {
        let info = Arc::new(TagInfos::default());
        let store = LocalStorage {
            attached_tag: SharedTagInfos::new(info),
            ..Default::default()
        };
        let mut recorder = CPURecorder::default();
        recorder.on_thread_created(thread::thread_id(), &store);
        let pid = thread::process_id();
        let tid = thread::thread_id();
        let prev_stat = &recorder.thread_stats.get(&tid).unwrap().stat;
        loop {
            let stat = thread::thread_stat(pid, tid).unwrap();
            let delta = stat.total_cpu_time() - prev_stat.total_cpu_time();
            if delta >= 0.001 {
                break;
            }
            heavy_job();
        }
        let mut records = Records::default();
        recorder.tick(&mut records);
        assert!(!records.records.is_empty());
        records
            .records
            .values()
            .for_each(|record| assert!(record.cpu_time > 0));
    }
}
