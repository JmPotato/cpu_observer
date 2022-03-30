mod thread;

use std::fmt::{self, Display};
use std::time::Duration;

use crate::worker::Builder as WorkerBuilder;
use crate::worker::Runnable;

pub struct Recorder {}

pub enum Task {}

impl Display for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Task")
    }
}

impl Runnable for Recorder {
    type Task = Task;

    fn run(&mut self, _task: Task) {
        unimplemented!()
    }

    fn on_timeout(&mut self) {
        unimplemented!()
    }

    fn get_interval(&self) -> Duration {
        Duration::from_millis(0)
    }

    fn shutdown(&mut self) {
        unimplemented!()
    }
}

fn init_recorder() {
    let recorder = Recorder {};
    let mut recorder_worker = WorkerBuilder::new("cpu-observer-recorder")
        .pending_capacity(256)
        .create()
        .lazy_build();
    recorder_worker.start_with_timer(recorder);
}
