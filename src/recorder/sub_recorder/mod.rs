pub mod cpu_recorder;

pub use cpu_recorder::CPURecorder;

use crate::local_storage::LocalStorage;
use crate::record::Records;

use super::thread::Pid;

pub trait SubRecorder: Send {
    fn tick(&mut self, _records: &mut Records);
    fn clean_up(&mut self) {}
    fn pause(&mut self) {}
    fn resume(&mut self) {}
    fn on_thread_created(&mut self, _id: Pid, _store: &LocalStorage) {}
}
