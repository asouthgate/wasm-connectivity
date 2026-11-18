use super::super::graph::NodeTaskList;
use super::{
    finish_node_run, next_ready_node, prepare_node_run, run_node, NodeExecutionState,
    NodeRunInputs, NodeStatus,
};
use crate::pipeline::{ExecutionOptions, Grid, NodeResult, Result};

pub(super) fn execute_with_workers(
    task_list: &NodeTaskList<'_>,
    states: &mut [NodeExecutionState],
    pending_edges: &mut [bool],
    grid: Grid,
    options: ExecutionOptions,
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
        let mut running_nodes = 0;
        while states
            .iter()
            .any(|state| state.status != NodeStatus::Complete)
        {
            while let Some(&worker_index) = idle_workers.last() {
                let Some(node_index) = next_ready_node(task_list, states, pending_edges, options)
                else {
                    break;
                };
                let inputs = prepare_node_run(task_list, states, pending_edges, node_index)?;
                workers.submit(worker_index, inputs)?;
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
                on_node_result,
            )?;
        }
        Ok(())
    })
}
