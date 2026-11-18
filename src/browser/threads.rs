use rayon::ThreadBuilder;
use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex},
};
use wasm_bindgen::prelude::*;

static STARTING_THREADS: Mutex<VecDeque<ThreadBuilder>> = Mutex::new(VecDeque::new());
static THREAD_AVAILABLE: Condvar = Condvar::new();

#[wasm_bindgen]
pub fn run_pipeline_thread() {
    let mut starting_threads = STARTING_THREADS.lock().unwrap();
    let starting_thread = loop {
        if let Some(thread) = starting_threads.pop_front() {
            break thread;
        }
        starting_threads = THREAD_AVAILABLE.wait(starting_threads).unwrap();
    };
    drop(starting_threads);
    starting_thread.run();
}

#[wasm_bindgen]
pub fn initialize_pipeline_threads(threads: usize) -> Result<(), String> {
    if threads < 2 {
        return Err("a threaded pipeline requires at least two workers".into());
    }

    // JavaScript starts the workers before Rayon builds the pool. Each
    // worker takes one entry from this queue, then runs that Rayon thread.
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .spawn_handler(|thread| {
            STARTING_THREADS.lock().unwrap().push_back(thread);
            THREAD_AVAILABLE.notify_one();
            Ok(())
        })
        .build_global()
        .map_err(|error| error.to_string())
}
