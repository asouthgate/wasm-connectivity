use super::super::graph::NodeTaskList;
use super::{
    ensure_complete, finish_node_run, next_ready_node, prepare_node_run, run_node, ExecutionTrace,
    NodeExecutionState, NodeRunInputs,
};
use crate::pipeline::{ExecutionOptions, Grid, NodeResult, Result};
use std::collections::VecDeque;

pub(super) fn execute_with_workers(
    task_list: &NodeTaskList<'_>,
    states: &mut [NodeExecutionState],
    pending_edges: &mut [bool],
    batches: impl Iterator<Item = VecDeque<usize>>,
    grid: Grid,
    options: ExecutionOptions,
    trace: &mut ExecutionTrace,
    on_node_result: &mut impl FnMut(NodeResult) -> Result<()>,
) -> Result<()> {
    let run_task = |inputs: NodeRunInputs| {
        let node_index = inputs.node_index;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_node(&task_list.nodes[node_index], inputs, grid, options.seed)
        }))
        .unwrap_or_else(|_| Err(format!("worker panicked while executing node {node_index}")));
        (node_index, result)
    };

    super::threads::with_workers(options.threads, run_task, |workers| {
        let mut idle_workers: Vec<_> = (0..options.threads).collect();
        for mut remaining_steps in batches {
            let mut running_nodes = 0;
            while !remaining_steps.is_empty() || running_nodes != 0 {
                while let Some(&worker_index) = idle_workers.last() {
                    let Some(node_index) =
                        next_ready_node(task_list, states, pending_edges, &mut remaining_steps)
                    else {
                        break;
                    };
                    let inputs = prepare_node_run(task_list, states, pending_edges, node_index)?;
                    workers.submit(worker_index, inputs)?;
                    trace.start_job(task_list, node_index, &states[node_index].active_edges);
                    idle_workers.pop();
                    running_nodes += 1;
                }
                if running_nodes == 0 {
                    return Err("task graph cannot make progress".into());
                }
                let (worker_index, (node_index, result)) = workers.receive()?;
                running_nodes -= 1;
                idle_workers.push(worker_index);
                finish_node_run(
                    task_list,
                    states,
                    pending_edges,
                    node_index,
                    result?,
                    options,
                    trace,
                    on_node_result,
                )?;
            }
        }
        ensure_complete(states)
    })
}
