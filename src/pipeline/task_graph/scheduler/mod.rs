// Execute the task graph built by builder.rs. Graph structures live in graph.rs.
use super::{
    graph::{NodeOperation, NodeTaskList, StreamingNode, TaskNode},
    operations::OperationEvaluator,
    results::emit_node_results,
    sampling::sample_operation,
};
use crate::pipeline::{ExecutionOptions, Grid, NodeOutputs, NodeResult, PipelineData, Result};
use std::collections::{BTreeMap, VecDeque};

mod schedule;
mod trace;
use schedule::Schedule;
use trace::ExecutionTrace;

#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
#[cfg_attr(target_arch = "wasm32", path = "wasm_threads.rs")]
#[cfg_attr(not(target_arch = "wasm32"), path = "native_threads.rs")]
mod threads;
#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
mod workers;

#[derive(Clone, Copy, PartialEq)]
enum NodeStatus {
    Waiting,
    Running,
    Complete,
}

struct NodeExecutionState {
    status: NodeStatus,
    outputs: NodeOutputs,
    streaming_node: Option<Box<dyn StreamingNode>>,
    active_edges: Vec<usize>,
}

struct NodeRunInputs {
    node_index: usize,
    data_by_port: BTreeMap<String, Vec<PipelineData>>,
    streaming_node: Option<Box<dyn StreamingNode>>,
    final_update: bool,
}

struct NodeUpdate {
    outputs: NodeOutputs,
    streaming_node: Option<Box<dyn StreamingNode>>,
}

fn initial_node_states(task_list: &mut NodeTaskList<'_>) -> Vec<NodeExecutionState> {
    task_list
        .nodes
        .iter_mut()
        .map(|node| NodeExecutionState {
            status: if matches!(node.operation, NodeOperation::Input) {
                NodeStatus::Complete
            } else {
                NodeStatus::Waiting
            },
            outputs: std::mem::take(&mut node.initial_outputs),
            streaming_node: match node.operation {
                NodeOperation::Streaming { create } => Some(create()),
                _ => None,
            },
            active_edges: Vec::new(),
        })
        .collect()
}

fn release_unused_outputs(
    task_list: &NodeTaskList<'_>,
    states: &mut [NodeExecutionState],
    pending_edges: &[bool],
    node_index: usize,
    trace: &mut ExecutionTrace,
) {
    let node = &task_list.nodes[node_index];
    states[node_index].outputs.retain(|port, _| {
        let still_used = node
            .output_edges
            .iter()
            .any(|&edge| pending_edges[edge] && &task_list.edges[edge].output_port == port);
        if !still_used {
            trace.release(node, port);
        }
        still_used
    });
}

fn ready_input_edges(
    task_list: &NodeTaskList<'_>,
    states: &[NodeExecutionState],
    pending_edges: &[bool],
    node_index: usize,
) -> Option<Vec<usize>> {
    let node = &task_list.nodes[node_index];
    if states[node_index].status != NodeStatus::Waiting {
        return None;
    }
    let ready = |edge: usize| {
        pending_edges[edge]
            && states[task_list.edges[edge].source_node].status == NodeStatus::Complete
    };
    match node.operation {
        NodeOperation::Streaming { .. } => node
            .input_edges
            .iter()
            .copied()
            .find(|&edge| ready(edge))
            .map(|edge| vec![edge]),
        NodeOperation::Calculate { .. } if node.input_edges.iter().all(|&edge| ready(edge)) => {
            Some(node.input_edges.clone())
        }
        _ => None,
    }
}

fn next_ready_node(
    task_list: &NodeTaskList<'_>,
    states: &[NodeExecutionState],
    pending_edges: &[bool],
    remaining_steps: &mut VecDeque<usize>,
) -> Option<usize> {
    let ready =
        |node_index| ready_input_edges(task_list, states, pending_edges, node_index).is_some();
    // Consume completed maps promptly instead of retaining them behind more calculations.
    let position = remaining_steps
        .iter()
        .position(|&node_index| {
            matches!(
                task_list.nodes[node_index].operation,
                NodeOperation::Streaming { .. }
            ) && ready(node_index)
        })
        .or_else(|| {
            remaining_steps
                .iter()
                .position(|&node_index| ready(node_index))
        })?;
    remaining_steps.remove(position)
}

fn prepare_node_run(
    task_list: &NodeTaskList<'_>,
    states: &mut [NodeExecutionState],
    pending_edges: &[bool],
    node_index: usize,
) -> Result<NodeRunInputs> {
    let edges = ready_input_edges(task_list, states, pending_edges, node_index)
        .ok_or("node inputs are not ready")?;
    let mut data_by_port: BTreeMap<String, Vec<PipelineData>> = BTreeMap::new();
    for &edge_index in &edges {
        let edge = &task_list.edges[edge_index];
        let value = states[edge.source_node]
            .outputs
            .get(&edge.output_port)
            .ok_or_else(|| format!("missing node output {}", edge.output_port))?;
        data_by_port
            .entry(edge.input_port.clone())
            .or_default()
            .push(value.clone());
    }
    let final_update = task_list.nodes[node_index]
        .input_edges
        .iter()
        .filter(|&&edge| pending_edges[edge])
        .count()
        == edges.len();
    states[node_index].active_edges = edges;
    states[node_index].status = NodeStatus::Running;
    Ok(NodeRunInputs {
        node_index,
        data_by_port,
        streaming_node: states[node_index].streaming_node.take(),
        final_update,
    })
}

fn run_node(
    node: &TaskNode<'_>,
    mut inputs: NodeRunInputs,
    grid: Grid,
    seed: u64,
) -> Result<NodeUpdate> {
    let outputs = match &node.operation {
        NodeOperation::Calculate {
            operation,
            sampled_parameters,
        } => {
            let sampled = sampled_parameters
                .map(|parameters| sample_operation(parameters, &node.name, node.sample, seed))
                .transpose()
                .map_err(|error| format!("sample {}, node {}: {error}", node.sample, node.name))?;
            OperationEvaluator { grid }
                .apply(sampled.as_ref().unwrap_or(operation), &inputs.data_by_port)
                .map_err(|error| format!("sample {}, node {}: {error}", node.sample, node.name))?
        }
        NodeOperation::Streaming { .. } => {
            let mut state = inputs
                .streaming_node
                .take()
                .ok_or("missing streaming node state")?;
            state.update(&inputs.data_by_port)?;
            if inputs.final_update {
                state.finish()?
            } else {
                inputs.streaming_node = Some(state);
                BTreeMap::new()
            }
        }
        NodeOperation::Input => return Err("input data does not need execution".into()),
    };
    Ok(NodeUpdate {
        outputs,
        streaming_node: inputs.streaming_node,
    })
}

fn finish_node_run(
    task_list: &NodeTaskList<'_>,
    states: &mut [NodeExecutionState],
    pending_edges: &mut [bool],
    node_index: usize,
    update: NodeUpdate,
    options: ExecutionOptions,
    trace: &mut ExecutionTrace,
    on_node_result: &mut impl FnMut(NodeResult) -> Result<()>,
) -> Result<()> {
    states[node_index].outputs = update.outputs;
    trace.finish_job(
        task_list,
        node_index,
        &states[node_index].active_edges,
        states[node_index].outputs.len(),
    );
    states[node_index].streaming_node = update.streaming_node;
    if states[node_index].streaming_node.is_some() {
        states[node_index].status = NodeStatus::Waiting;
    } else {
        states[node_index].status = NodeStatus::Complete;
        emit_node_results(
            &task_list.nodes[node_index],
            &states[node_index].outputs,
            options,
            on_node_result,
        )?;
    }
    for edge_index in std::mem::take(&mut states[node_index].active_edges) {
        pending_edges[edge_index] = false;
        release_unused_outputs(
            task_list,
            states,
            pending_edges,
            task_list.edges[edge_index].source_node,
            trace,
        );
    }
    release_unused_outputs(task_list, states, pending_edges, node_index, trace);
    Ok(())
}

fn ensure_complete(states: &[NodeExecutionState]) -> Result<()> {
    if states
        .iter()
        .any(|state| state.status != NodeStatus::Complete)
    {
        return Err("task graph cannot make progress".into());
    }
    Ok(())
}

/// Run ready nodes. Each streaming update owns its state until that update finishes.
pub(in crate::pipeline) fn execute_node_task_list(
    mut task_list: NodeTaskList<'_>,
    grid: Grid,
    options: ExecutionOptions,
    verbose: bool,
    mut on_node_result: impl FnMut(NodeResult) -> Result<()>,
) -> Result<()> {
    let mut states = initial_node_states(&mut task_list);
    let mut trace = ExecutionTrace::new(verbose, &task_list, &states);
    let mut pending_edges = vec![true; task_list.edges.len()];
    for node_index in 0..task_list.nodes.len() {
        if states[node_index].status == NodeStatus::Complete {
            emit_node_results(
                &task_list.nodes[node_index],
                &states[node_index].outputs,
                options,
                &mut on_node_result,
            )?;
            release_unused_outputs(
                &task_list,
                &mut states,
                &pending_edges,
                node_index,
                &mut trace,
            );
        }
    }
    let batches = Schedule::generate(&task_list, options.samples).batches(options.threads);
    if options.threads == 1 {
        for mut remaining_steps in batches {
            while !remaining_steps.is_empty() {
                let node_index =
                    next_ready_node(&task_list, &states, &pending_edges, &mut remaining_steps)
                        .ok_or("task graph cannot make progress")?;
                let inputs = prepare_node_run(&task_list, &mut states, &pending_edges, node_index)?;
                trace.start_job(&task_list, node_index, &states[node_index].active_edges);
                let update = run_node(
                    &task_list.nodes[inputs.node_index],
                    inputs,
                    grid,
                    options.seed,
                )?;
                finish_node_run(
                    &task_list,
                    &mut states,
                    &mut pending_edges,
                    node_index,
                    update,
                    options,
                    &mut trace,
                    &mut on_node_result,
                )?;
            }
        }
        return ensure_complete(&states);
    }
    #[cfg(all(target_arch = "wasm32", not(feature = "wasm-threads")))]
    return Err("Wasm execution requires one thread".into());
    #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
    workers::execute_with_workers(
        &task_list,
        &mut states,
        &mut pending_edges,
        batches,
        grid,
        options,
        &mut trace,
        &mut on_node_result,
    )
}

#[cfg(test)]
mod tests {
    use super::super::builder::build_node_task_list;
    use super::*;
    use crate::pipeline::{OutputLayer, PipelineGraph};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn options() -> ExecutionOptions {
        ExecutionOptions {
            samples: 3,
            threads: 2,
            seed: 0,
            all_layers: false,
            statistics: true,
        }
    }

    fn grid() -> Grid {
        Grid {
            nrows: 1,
            ncols: 1,
            pixel_width: 1.0,
        }
    }

    fn graph() -> PipelineGraph {
        PipelineGraph::from_json(
            &json!({
                "name":"streaming", "inputs":{"data":{"type":"raster_ranked"}},
                "nodes":{"result":{"op":"power_response","inputs":"inputs.data","parameters":{
                    "input_range":[0,1], "base_range":[0,1], "exponent":1,
                    "scale":{"sample":{"values":[1,2,3]}}, "offset":0
                }}}, "output":"nodes.result"
            })
            .to_string(),
        )
        .unwrap()
    }

    #[test]
    fn schedule_reuses_calculation_and_streaming_steps_in_partial_batch() {
        let graph = graph();
        let task_list = build_node_task_list(
            &graph,
            HashMap::from([("data".into(), PipelineData::raster(vec![1.0]))]),
            options(),
            false,
        )
        .unwrap();
        let batches: Vec<Vec<_>> = Schedule::generate(&task_list, options().samples)
            .batches(options().threads)
            .map(|batch| {
                batch
                    .into_iter()
                    .map(|node_index| {
                        let node = &task_list.nodes[node_index];
                        (node.name.as_str(), node.sample)
                    })
                    .collect()
            })
            .collect();
        assert_eq!(
            batches,
            vec![
                vec![
                    ("result", 0),
                    ("statistics", 0),
                    ("result", 1),
                    ("statistics", 0)
                ],
                vec![("result", 2), ("statistics", 0)],
            ]
        );
    }

    #[test]
    fn streaming_update_runs_before_other_samples_and_releases_its_input() {
        let graph = graph();
        let mut task_list = build_node_task_list(
            &graph,
            HashMap::from([("data".into(), PipelineData::raster(vec![1.0]))]),
            options(),
            false,
        )
        .unwrap();
        let mut states = initial_node_states(&mut task_list);
        let mut pending_edges = vec![true; task_list.edges.len()];
        let stream_index = task_list
            .nodes
            .iter()
            .position(|node| matches!(node.operation, NodeOperation::Streaming { .. }))
            .unwrap();
        let mut summary = None;
        for sample in [1, 2, 0] {
            let calculation = task_list
                .nodes
                .iter()
                .position(|node| {
                    matches!(node.operation, NodeOperation::Calculate { .. })
                        && node.sample == sample
                })
                .unwrap();
            let inputs =
                prepare_node_run(&task_list, &mut states, &pending_edges, calculation).unwrap();
            let update = run_node(&task_list.nodes[calculation], inputs, grid(), 0).unwrap();
            let PipelineData::Raster(values) = &update.outputs["data"] else {
                panic!()
            };
            let sample_data = Arc::downgrade(values);
            finish_node_run(
                &task_list,
                &mut states,
                &mut pending_edges,
                calculation,
                update,
                options(),
                &mut ExecutionTrace::default(),
                &mut |_| Ok(()),
            )
            .unwrap();
            let inputs =
                prepare_node_run(&task_list, &mut states, &pending_edges, stream_index).unwrap();
            assert_eq!(inputs.data_by_port["data"].len(), 1);
            assert!(
                ready_input_edges(&task_list, &states, &pending_edges, stream_index).is_none(),
                "a running streaming node cannot be scheduled twice"
            );
            let update = run_node(&task_list.nodes[stream_index], inputs, grid(), 0).unwrap();
            finish_node_run(
                &task_list,
                &mut states,
                &mut pending_edges,
                stream_index,
                update,
                options(),
                &mut ExecutionTrace::default(),
                &mut |result| {
                    summary = Some(result);
                    Ok(())
                },
            )
            .unwrap();
            assert!(
                sample_data.upgrade().is_none(),
                "the processed map must not remain in memory"
            );
            if sample != 0 {
                assert!(summary.is_none());
                assert!(states[stream_index].status == NodeStatus::Waiting);
            }
        }
        let summary = summary.unwrap();
        assert!(summary.sample.is_none());
        assert_eq!(summary.outputs["mean"].values().unwrap(), &[2.0]);
        assert_eq!(summary.outputs["variance"].values().unwrap(), &[1.0]);
        assert!(pending_edges.iter().all(|pending| !pending));
    }

    struct SumInputs {
        total: f64,
    }
    impl SumInputs {
        fn create() -> Box<dyn StreamingNode> {
            Box::new(Self { total: 0.0 })
        }
    }
    impl StreamingNode for SumInputs {
        fn update(&mut self, inputs: &BTreeMap<String, Vec<PipelineData>>) -> Result<()> {
            self.total += inputs["data"][0].values()?[0];
            Ok(())
        }
        fn finish(self: Box<Self>) -> Result<NodeOutputs> {
            Ok(BTreeMap::from([(
                "sum".into(),
                PipelineData::raster(vec![self.total]),
            )]))
        }
    }

    #[test]
    fn streaming_node_can_run_an_operation_other_than_statistics() {
        let mut task_list = NodeTaskList {
            nodes: Vec::new(),
            edges: Vec::new(),
        };
        let source = task_list.add_node(
            "source",
            0,
            true,
            NodeOperation::Input,
            BTreeMap::from([("data".into(), PipelineData::raster(vec![2.0]))]),
        );
        let destination = task_list.add_node(
            "sum",
            0,
            false,
            NodeOperation::Streaming {
                create: SumInputs::create,
            },
            BTreeMap::new(),
        );
        task_list.nodes[destination]
            .result_layers
            .insert("data".into(), OutputLayer::Resistance);
        for _ in 0..3 {
            task_list.add_edge(source, destination, "data".into(), "data".into());
        }
        let mut count = 0;
        execute_node_task_list(task_list, grid(), options(), false, |result| {
            assert_eq!(result.outputs["sum"].values()?, &[6.0]);
            count += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(count, 1);
    }
}
