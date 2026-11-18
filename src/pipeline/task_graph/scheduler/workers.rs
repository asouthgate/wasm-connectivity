use super::{
    finish_node_run, next_ready_node, prepare_node_run, run_node, NodeExecutionState,
    NodeRunInputs, NodeStatus,
};
use super::super::graph::NodeTaskList;
use crate::pipeline::{ExecutionOptions, Grid, NodeResult, Result};

pub(super) fn execute_with_workers(
    task_list: &NodeTaskList<'_>,
    states: &mut [NodeExecutionState],
    pending_edges: &mut [bool],
    grid: Grid,
    options: ExecutionOptions,
    on_node_result: &mut impl FnMut(NodeResult) -> Result<()>,
) -> Result<()> {
    std::thread::scope(|scope| {
        let (completed_sender, completed_receiver) = std::sync::mpsc::channel();
        let mut worker_senders = Vec::new();
        let mut workers = Vec::new();
        for worker_index in 0..options.threads {
            let (sender, receiver) = std::sync::mpsc::sync_channel::<NodeRunInputs>(1);
            let completed_sender = completed_sender.clone();
            workers.push(scope.spawn(move || {
                while let Ok(inputs) = receiver.recv() {
                    let node_index = inputs.node_index;
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run_node(&task_list.nodes[node_index], inputs, grid, options.seed)
                    }))
                    .unwrap_or_else(|_| {
                        Err(format!("worker panicked while executing node {node_index}"))
                    });
                    if completed_sender
                        .send((worker_index, node_index, result))
                        .is_err()
                    {
                        break;
                    }
                }
            }));
            worker_senders.push(sender);
        }
        drop(completed_sender);
        let result = (|| {
            let mut idle_workers: Vec<_> = (0..options.threads).collect();
            let mut running_nodes = 0;
            while states
                .iter()
                .any(|state| state.status != NodeStatus::Complete)
            {
                while let Some(&worker_index) = idle_workers.last() {
                    let Some(node_index) =
                        next_ready_node(task_list, states, pending_edges, options)
                    else {
                        break;
                    };
                    let inputs = prepare_node_run(task_list, states, pending_edges, node_index)?;
                    worker_senders[worker_index]
                        .send(inputs)
                        .map_err(|_| "task worker disconnected")?;
                    idle_workers.pop();
                    running_nodes += 1;
                }
                if running_nodes == 0 {
                    return Err("task graph cannot make progress".into());
                }
                let (worker_index, node_index, result) = completed_receiver
                    .recv()
                    .map_err(|_| "task workers disconnected")?;
                running_nodes -= 1;
                idle_workers.push(worker_index);
                finish_node_run(
                    task_list,
                    states,
                    pending_edges,
                    node_index,
                    result?,
                    options,
                    on_node_result,
                )?;
            }
            Ok(())
        })();
        drop(worker_senders);
        drop(completed_receiver);
        for worker in workers {
            if worker.join().is_err() {
                return Err("task worker panicked".into());
            }
        }
        result
    })
}
