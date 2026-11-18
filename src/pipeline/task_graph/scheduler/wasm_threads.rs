use crate::pipeline::Result;
use std::sync::mpsc::{self, Receiver};

pub(super) struct WorkerPool<'a, Task, Completion> {
    submit_task: &'a dyn Fn(usize, Task),
    completions: Receiver<(usize, Completion)>,
}

impl<Task, Completion> WorkerPool<'_, Task, Completion> {
    pub(super) fn submit(&self, worker_index: usize, task: Task) -> Result<()> {
        (self.submit_task)(worker_index, task);
        Ok(())
    }

    pub(super) fn receive(&self) -> Result<(usize, Completion)> {
        self.completions
            .recv()
            .map_err(|_| "task workers disconnected".into())
    }
}

pub(super) fn with_workers<Task: Send, Completion: Send>(
    worker_count: usize,
    run_task: impl Fn(Task) -> Completion + Sync,
    schedule: impl FnOnce(&WorkerPool<'_, Task, Completion>) -> Result<()>,
) -> Result<()> {
    if rayon::current_num_threads() < worker_count {
        return Err("initialize the Wasm thread pool before running the pipeline".into());
    }

    // Keep the scheduler on its own worker. Only submitted tasks run in the pool.
    rayon::in_place_scope(|scope| {
        let (completed_sender, completions) = mpsc::channel();
        let run_task = &run_task;
        let submit_task = |worker_index, task| {
            let completed_sender = completed_sender.clone();
            scope.spawn(move |_| {
                let completion = run_task(task);
                let _ = completed_sender.send((worker_index, completion));
            });
        };
        schedule(&WorkerPool {
            submit_task: &submit_task,
            completions,
        })
    })
}
