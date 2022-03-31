use core::fmt;
use std::{
    error::Error,
    fmt::{Debug, Display, Formatter},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use futures::{
    channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender},
    StreamExt,
};
use futures_timer::Delay;
use yatp::{task::future::TaskCell, Remote, ThreadPool};

// Runnable is a trait that represents the methods a runner need to implement.
// Task and the corresponding handler are defined by the implementer.
pub trait Runnable: Send {
    type Task: Display + Send + 'static;

    fn run(&mut self, _: Self::Task) {
        unimplemented!()
    }
    fn on_timeout(&mut self);
    fn get_interval(&self) -> Duration;
    fn shutdown(&mut self) {}
}

// Msg is an internal type to distinguish different kinds of events.
pub enum Msg<T: Display + Send> {
    Task(T),
    Timeout,
}

#[derive(Eq, PartialEq)]
pub enum ScheduleError<T> {
    Stopped(T),
    Full(T),
}

impl<T> Error for ScheduleError<T> {}

impl<T> Display for ScheduleError<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let msg = match *self {
            ScheduleError::Stopped(_) => "scheduler channel has been closed",
            ScheduleError::Full(_) => "scheduler channel is full",
        };
        write!(f, "{}", msg)
    }
}

impl<T> Debug for ScheduleError<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

// Scheduler is used to dispatch the tasks to the worker.
// It's a wrapper of the task sender channel.
pub struct Scheduler<T: Display + Send> {
    task_counter: Arc<AtomicUsize>,
    task_sender: UnboundedSender<Msg<T>>,
    pending_capacity: usize,
}

impl<T: Display + Send> Scheduler<T> {
    fn new(
        task_counter: Arc<AtomicUsize>,
        task_sender: UnboundedSender<Msg<T>>,
        pending_capacity: usize,
    ) -> Self {
        Self {
            task_counter,
            task_sender,
            pending_capacity,
        }
    }

    /// Schedules a task to run.
    ///
    /// If the worker is stopped or number pending tasks exceeds capacity, an error will return.
    pub fn schedule(&self, task: T) -> Result<(), ScheduleError<T>> {
        if self.task_counter.load(Ordering::Acquire) >= self.pending_capacity {
            return Err(ScheduleError::Full(task));
        }
        self.schedule_force(task)
    }

    /// Schedules a task to run.
    ///
    /// Different from the `schedule` function, the task will still be scheduled
    /// if pending task number exceeds capacity.
    pub fn schedule_force(&self, task: T) -> Result<(), ScheduleError<T>> {
        self.task_counter.fetch_add(1, Ordering::SeqCst);
        if let Err(e) = self.task_sender.unbounded_send(Msg::Task(task)) {
            if let Msg::Task(t) = e.into_inner() {
                self.task_counter.fetch_sub(1, Ordering::SeqCst);
                return Err(ScheduleError::Stopped(t));
            }
        }
        Ok(())
    }

    pub fn _stop(&self) {
        self.task_sender.close_channel();
    }
}

impl<T: Display + Send> Clone for Scheduler<T> {
    fn clone(&self) -> Scheduler<T> {
        Scheduler {
            task_counter: Arc::clone(&self.task_counter),
            task_sender: self.task_sender.clone(),
            pending_capacity: self.pending_capacity,
        }
    }
}

// Builder is used to build a worker with the given parameters.
pub struct Builder<S: Into<String>> {
    name: S,
    thread_count: usize,
    pending_capacity: usize,
}

impl<S: Into<String>> Builder<S> {
    pub fn new(name: S) -> Self {
        Builder {
            name,
            thread_count: 1,
            pending_capacity: usize::MAX,
        }
    }

    /// Pending tasks won't exceed `pending_capacity`.
    #[must_use]
    pub fn pending_capacity(mut self, pending_capacity: usize) -> Self {
        self.pending_capacity = pending_capacity;
        self
    }

    pub fn create(self) -> Worker {
        let pool = yatp::Builder::new(self.name.into())
            .core_thread_count(self.thread_count)
            .min_thread_count(self.thread_count)
            .max_thread_count(self.thread_count)
            .build_future_pool();
        let remote = pool.remote().clone();
        Worker {
            _pool: Arc::new(Mutex::new(Some(pool))),
            remote,
            task_counter: Arc::new(AtomicUsize::new(0)),
            pending_capacity: self.pending_capacity,
        }
    }
}

#[derive(Clone)]
pub struct Worker {
    _pool: Arc<Mutex<Option<ThreadPool<TaskCell>>>>,
    remote: Remote<TaskCell>,
    task_counter: Arc<AtomicUsize>,
    pending_capacity: usize,
}

impl Worker {
    pub fn lazy_build<T: Display + Send + 'static>(&self) -> LazyWorker<T> {
        let (task_sender, task_receiver) = unbounded();
        LazyWorker {
            worker: self.clone(),
            scheduler: Scheduler::new(
                self.task_counter.clone(),
                task_sender,
                self.pending_capacity,
            ),
            task_receiver: Some(task_receiver),
        }
    }

    fn start_with_timer_impl<R>(
        &self,
        mut runner: R,
        task_sender: UnboundedSender<Msg<R::Task>>,
        mut task_receiver: UnboundedReceiver<Msg<R::Task>>,
    ) where
        R: Runnable + 'static,
    {
        let counter = self.task_counter.clone();
        let timeout = runner.get_interval();
        self.start_timer(task_sender, timeout);
        self.remote.spawn(async move {
            while let Some(msg) = task_receiver.next().await {
                match msg {
                    Msg::Task(task) => {
                        runner.run(task);
                        counter.fetch_sub(1, Ordering::SeqCst);
                    }
                    Msg::Timeout => {
                        runner.on_timeout();
                    }
                }
            }
        });
    }

    fn start_timer<T>(&self, task_sender: UnboundedSender<Msg<T>>, timeout: Duration)
    where
        T: Display + Send + 'static,
    {
        self.remote.spawn(async move {
            loop {
                Delay::new(timeout).await;
                if let Ok(()) = task_sender.unbounded_send(Msg::Timeout) {
                    continue;
                }
            }
        });
    }

    fn _stop(&self) {
        if let Some(pool) = self._pool.lock().unwrap().take() {
            pool.shutdown();
        }
    }
}

// LazyWorker is the [`Worker`] that initialized a [`Scheduler`] lazily.
pub struct LazyWorker<T: Display + Send + 'static> {
    worker: Worker,
    scheduler: Scheduler<T>,
    task_receiver: Option<UnboundedReceiver<Msg<T>>>,
}

impl<T: Display + Send + 'static> LazyWorker<T> {
    pub fn start_with_timer<R: 'static + Runnable<Task = T>>(&mut self, runner: R) -> bool {
        if let Some(receiver) = self.task_receiver.take() {
            self.worker
                .start_with_timer_impl(runner, self.scheduler.task_sender.clone(), receiver);
            return true;
        }
        false
    }

    // [`Scheduler`] could be cloned to dispatch the task concurrently at different places.
    pub fn scheduler(&self) -> Scheduler<T> {
        self.scheduler.clone()
    }

    pub fn _stop(&mut self) {
        self.scheduler._stop();
        self.worker._stop()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::mpsc::{channel, Sender},
        thread::sleep,
    };

    use super::*;

    const TEST_TIMEOUT: Duration = Duration::from_millis(100);
    const TEST_DURATION: Duration = Duration::from_secs(1);

    struct TestRunner {
        sender: Sender<i32>,
    }

    impl Runnable for TestRunner {
        type Task = i32;

        fn run(&mut self, _task: Self::Task) {}

        fn on_timeout(&mut self) {
            self.sender.send(42).unwrap()
        }

        fn get_interval(&self) -> Duration {
            Duration::from_millis(100)
        }
    }

    #[test]
    fn test_worker_timer() {
        let (sender, receiver) = channel();
        {
            let runner = TestRunner { sender };
            let mut lazy_worker = Builder::new("test-worker-timer").create().lazy_build();
            lazy_worker.start_with_timer(runner);
            sleep(Duration::from_secs(1));
        }
        let mut received_counter = 0;
        while let Ok(42) = receiver.recv() {
            received_counter += 1;
        }
        assert_eq!(
            received_counter,
            TEST_DURATION
                .as_millis()
                .checked_div(TEST_TIMEOUT.as_millis())
                .unwrap()
                - 1
        );
    }
}
