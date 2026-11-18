use crate::pipeline::Result;
use std::sync::mpsc::{self, Receiver, SyncSender};

pub(super) struct WorkerPool<Task, Completion> {
    task_senders: Vec<SyncSender<Task>>,
    completions: Receiver<(usize, Completion)>,
}

impl<Task, Completion> WorkerPool<Task, Completion> {
    pub(super) fn submit(&self, worker_index: usize, task: Task) -> Result<()> {
        self.task_senders[worker_index]
            .send(task)
            .map_err(|_| "task worker disconnected".into())
    }

    pub(super) fn receive(&self) -> Result<(usize, Completion)> {
        self.completions
            .recv()
            .map_err(|_| "task workers disconnected".into())
    }
}

/// Run the scheduler with scoped workers. All workers finish before returning,
/// even when scheduling fails, so tasks can borrow from the execution graph.
pub(super) fn with_workers<Task: Send, Completion: Send>(
    worker_count: usize,
    run_task: impl Fn(Task) -> Completion + Sync,
    schedule: impl FnOnce(&WorkerPool<Task, Completion>) -> Result<()>,
) -> Result<()> {
    std::thread::scope(|scope| {
        let (completed_sender, completions) = mpsc::channel();
        let mut task_senders = Vec::with_capacity(worker_count);
        let mut workers = Vec::with_capacity(worker_count);
        for worker_index in 0..worker_count {
            let (sender, receiver) = mpsc::sync_channel(1);
            let completed_sender = completed_sender.clone();
            let run_task = &run_task;
            workers.push(scope.spawn(move || {
                while let Ok(task) = receiver.recv() {
                    let completion = run_task(task);
                    if completed_sender.send((worker_index, completion)).is_err() {
                        break;
                    }
                }
            }));
            task_senders.push(sender);
        }
        drop(completed_sender);

        let pool = WorkerPool {
            task_senders,
            completions,
        };
        let result = schedule(&pool);
        drop(pool);
        for worker in workers {
            if worker.join().is_err() {
                return Err("task worker panicked".into());
            }
        }
        result
    })
}
