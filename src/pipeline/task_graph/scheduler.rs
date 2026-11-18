// This contains code for the scheduler that executes a pipeline DAG with sampling.
// We first build a bigger graph: nodes whose results stay the same across samples
// appear once, and nodes whose results change appear once for each sample.
// We then connect those nodes and sort them by dependency.
// For this bigger graph, we refer to TaskNodes and TaskNodeEdges, and the graph is a NodeTaskList.
//
// Threads execute nodes when their input data is ready. Connections keep data
// available until every node that reads it has finished, after which it can be freed.
// Requested results are saved when the operation producing them finishes.
//
// There are three types of nodes: input nodes, calculation nodes, and streaming nodes.
// Input nodes hold supplied data. Calculation nodes run an operation once.
// Streaming nodes retain their state and process one available input at a time,
// returning the worker thread after each update. Statistics use streaming nodes
// to update means and variances without retaining every sample map.
use super::{operations::OperationEvaluator, statistics::StatisticsAccumulator};
use crate::pipeline::{
    model::{parameters::SampledParameters, schema::{Node, Operation}},
    ExecutionOptions, Grid, NodeOutputs, NodeResult, OutputLayer, PipelineData,
    PipelineDataReference, PipelineGraph, Result,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

pub(super) trait StreamingNode: Send {
    fn update(&mut self, inputs: &BTreeMap<String, Vec<PipelineData>>) -> Result<()>;
    fn finish(self: Box<Self>) -> Result<NodeOutputs>;
}

#[derive(Clone, Copy)]
enum NodeOperation<'a> {
    Input,
    Calculate {
        operation: &'a Operation,
        sampled_parameters: Option<&'a SampledParameters>,
    },
    Streaming {
        create: fn() -> Box<dyn StreamingNode>,
    },
}

struct TaskNode<'a> {
    name: String,
    sample: usize,
    is_shared: bool,
    operation: NodeOperation<'a>,
    input_edges: Vec<usize>,
    output_edges: Vec<usize>,
    initial_outputs: NodeOutputs,
    result_layers: BTreeMap<String, OutputLayer>,
}

struct NodeEdge {
    source_node: usize,
    destination_node: usize,
    output_port: String,
    input_port: String,
}

pub(in crate::pipeline) struct NodeTaskList<'a> {
    nodes: Vec<TaskNode<'a>>,
    edges: Vec<NodeEdge>,
}

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

/// Create nodes whose results stay the same across samples once, and create the
/// other nodes for each sample. Connect the nodes and sort them by dependency.
pub(in crate::pipeline) fn build_node_task_list<'a>(
    graph: &'a PipelineGraph,
    inputs: HashMap<String, PipelineData>,
    options: ExecutionOptions,
    current_output: Option<PipelineDataReference>,
) -> Result<NodeTaskList<'a>> {
    let mut task_list = NodeTaskList {
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let mut node_indices = HashMap::new();
    let varying_nodes = graph.sample_dependent_nodes();
    task_list.add_input_nodes(graph, inputs, &mut node_indices)?;
    let (constant_node_names, varying_node_names): (Vec<_>, Vec<_>) = graph
        .order
        .iter()
        .partition(|name| !varying_nodes.contains(*name));
    task_list.add_calculation_nodes(graph, &constant_node_names, 0, true, &mut node_indices);
    for sample in 0..options.samples {
        task_list.add_calculation_nodes(
            graph,
            &varying_node_names,
            sample,
            false,
            &mut node_indices,
        );
    }
    task_list.connect_calculation_inputs(graph, &varying_nodes, &node_indices);
    task_list.connect_results_and_statistics(
        graph,
        options,
        current_output.as_ref(),
        &varying_nodes,
        &node_indices,
    );
    sort_nodes_topologically(task_list)
}

/// Run ready nodes. Each streaming update owns its state until that update finishes.
pub(in crate::pipeline) fn execute_node_task_list(
    mut task_list: NodeTaskList<'_>,
    grid: Grid,
    options: ExecutionOptions,
    mut on_node_result: impl FnMut(NodeResult) -> Result<()>,
) -> Result<()> {
    let mut states = initial_node_states(&mut task_list);
    let mut pending_edges = vec![true; task_list.edges.len()];
    for node_index in 0..task_list.nodes.len() {
        if states[node_index].status == NodeStatus::Complete {
            emit_node_results(
                &task_list.nodes[node_index],
                &states[node_index].outputs,
                options,
                &mut on_node_result,
            )?;
            release_unused_outputs(&task_list, &mut states, &pending_edges, node_index);
        }
    }
    if options.threads == 1 {
        while states
            .iter()
            .any(|state| state.status != NodeStatus::Complete)
        {
            let node_index = next_ready_node(&task_list, &states, &pending_edges, options)
                .ok_or("task graph cannot make progress")?;
            let inputs = prepare_node_run(&task_list, &mut states, &pending_edges, node_index)?;
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
                &mut on_node_result,
            )?;
        }
        return Ok(());
    }
    #[cfg(target_arch = "wasm32")]
    return Err("Wasm execution requires one thread".into());
    #[cfg(not(target_arch = "wasm32"))]
    execute_with_workers(
        &task_list,
        &mut states,
        &mut pending_edges,
        grid,
        options,
        &mut on_node_result,
    )
}

impl<'a> NodeTaskList<'a> {
    fn add_node(
        &mut self,
        name: &str,
        sample: usize,
        is_shared: bool,
        operation: NodeOperation<'a>,
        initial_outputs: NodeOutputs,
    ) -> usize {
        let index = self.nodes.len();
        self.nodes.push(TaskNode {
            name: name.into(),
            sample,
            is_shared,
            operation,
            input_edges: Vec::new(),
            output_edges: Vec::new(),
            initial_outputs,
            result_layers: BTreeMap::new(),
        });
        index
    }

    fn add_edge(
        &mut self,
        source_node: usize,
        destination_node: usize,
        output_port: String,
        input_port: String,
    ) {
        let index = self.edges.len();
        self.edges.push(NodeEdge {
            source_node,
            destination_node,
            output_port,
            input_port,
        });
        self.nodes[source_node].output_edges.push(index);
        self.nodes[destination_node].input_edges.push(index);
    }

    fn add_input_nodes(
        &mut self,
        graph: &PipelineGraph,
        mut inputs: HashMap<String, PipelineData>,
        node_indices: &mut HashMap<(usize, String), usize>,
    ) -> Result<()> {
        for name in graph.inputs.keys() {
            let value = inputs
                .remove(name)
                .ok_or_else(|| format!("missing input: {name}"))?;
            let index = self.add_node(
                name,
                0,
                true,
                NodeOperation::Input,
                BTreeMap::from([("data".into(), value)]),
            );
            node_indices.insert((0, name.clone()), index);
        }
        Ok(())
    }

    fn add_calculation_nodes(
        &mut self,
        graph: &'a PipelineGraph,
        node_names: &[&String],
        sample: usize,
        is_shared: bool,
        node_indices: &mut HashMap<(usize, String), usize>,
    ) {
        for name in node_names {
            let Node {
                operation,
                sampling,
                ..
            } = &graph.nodes[*name];
            let index = self.add_node(
                name,
                sample,
                is_shared,
                NodeOperation::Calculate {
                    operation,
                    sampled_parameters: sampling.as_ref(),
                },
                BTreeMap::new(),
            );
            node_indices.insert((sample, (*name).clone()), index);
        }
    }

    fn connect_calculation_inputs(
        &mut self,
        graph: &PipelineGraph,
        varying_nodes: &BTreeSet<String>,
        node_indices: &HashMap<(usize, String), usize>,
    ) {
        for node_index in graph.inputs.len()..self.nodes.len() {
            let name = self.nodes[node_index].name.clone();
            let sample = self.nodes[node_index].sample;
            for (input_port, connection) in &graph.nodes[&name].inputs {
                for reference in connection.references() {
                    let (source, port) =
                        resolve_model_connection(reference, sample, varying_nodes, node_indices);
                    self.add_edge(source, node_index, port, input_port.clone());
                }
            }
        }
    }

    fn connect_results_and_statistics(
        &mut self,
        graph: &PipelineGraph,
        options: ExecutionOptions,
        current_output: Option<&PipelineDataReference>,
        varying_nodes: &BTreeSet<String>,
        node_indices: &HashMap<(usize, String), usize>,
    ) {
        let mut layers = vec![(OutputLayer::Resistance, graph.output.clone())];
        if let Some(reference) = current_output {
            layers.push((OutputLayer::Current, reference.clone()));
        }
        if options.all_layers {
            for name in &graph.named_nodes {
                for &port in graph.nodes[name].operation.output_names() {
                    if graph.output.selects_node(name, port) {
                        continue;
                    }
                    layers.push((
                        OutputLayer::Intermediate {
                            node_name: name.clone(),
                            port_name: port.into(),
                        },
                        PipelineDataReference::Node {
                            from_node: name.clone(),
                            output: Some(port.into()),
                        },
                    ));
                }
            }
        }
        for (layer, reference) in layers {
            let streaming_node = options.statistics.then(|| {
                let index = self.add_node(
                    "statistics",
                    0,
                    false,
                    NodeOperation::Streaming {
                        create: StatisticsAccumulator::create,
                    },
                    BTreeMap::new(),
                );
                self.nodes[index]
                    .result_layers
                    .insert("data".into(), layer.clone());
                index
            });
            for sample in 0..options.samples {
                let (source, port) =
                    resolve_model_connection(&reference, sample, varying_nodes, node_indices);
                self.nodes[source]
                    .result_layers
                    .insert(port.clone(), layer.clone());
                if let Some(destination) = streaming_node {
                    self.add_edge(source, destination, port, "data".into());
                }
            }
        }
    }
}

fn resolve_model_connection(
    reference: &PipelineDataReference,
    sample: usize,
    varying_nodes: &BTreeSet<String>,
    node_indices: &HashMap<(usize, String), usize>,
) -> (usize, String) {
    match reference {
        PipelineDataReference::Input { from_input } => {
            (node_indices[&(0, from_input.clone())], "data".into())
        }
        PipelineDataReference::Node { from_node, output } => {
            let sample = if varying_nodes.contains(from_node) {
                sample
            } else {
                0
            };
            (
                node_indices[&(sample, from_node.clone())],
                output.as_deref().unwrap_or("data").into(),
            )
        }
    }
}

fn sort_nodes_topologically(mut task_list: NodeTaskList<'_>) -> Result<NodeTaskList<'_>> {
    let mut remaining_inputs: Vec<_> = task_list
        .nodes
        .iter()
        .map(|node| node.input_edges.len())
        .collect();
    let mut ready_nodes: VecDeque<_> = (0..task_list.nodes.len())
        .filter(|&index| remaining_inputs[index] == 0)
        .collect();
    let mut sorted_indices = Vec::new();
    while let Some(node_index) = ready_nodes.pop_front() {
        sorted_indices.push(node_index);
        for &edge_index in &task_list.nodes[node_index].output_edges {
            let destination = task_list.edges[edge_index].destination_node;
            remaining_inputs[destination] -= 1;
            if remaining_inputs[destination] == 0 {
                ready_nodes.push_back(destination);
            }
        }
    }
    if sorted_indices.len() != task_list.nodes.len() {
        return Err("expanded task graph contains a cycle".into());
    }
    let mut new_indices = vec![0; task_list.nodes.len()];
    let mut old_nodes: Vec<_> = task_list.nodes.into_iter().map(Some).collect();
    task_list.nodes = sorted_indices
        .into_iter()
        .enumerate()
        .map(|(new_index, old_index)| {
            new_indices[old_index] = new_index;
            old_nodes[old_index]
                .take()
                .expect("node appears once in topological order")
        })
        .collect();
    for edge in &mut task_list.edges {
        edge.source_node = new_indices[edge.source_node];
        edge.destination_node = new_indices[edge.destination_node];
    }
    Ok(task_list)
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
) {
    states[node_index].outputs.retain(|port, _| {
        task_list.nodes[node_index]
            .output_edges
            .iter()
            .any(|&edge| pending_edges[edge] && &task_list.edges[edge].output_port == port)
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
    options: ExecutionOptions,
) -> Option<usize> {
    // Consume completed maps promptly instead of retaining them behind more calculations.
    if let Some(index) = (0..task_list.nodes.len()).find(|&index| {
        matches!(
            task_list.nodes[index].operation,
            NodeOperation::Streaming { .. }
        ) && ready_input_edges(task_list, states, pending_edges, index).is_some()
    }) {
        return Some(index);
    }
    let first_unfinished_sample = task_list
        .nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            matches!(node.operation, NodeOperation::Calculate { .. })
                && states[*index].status != NodeStatus::Complete
        })
        .map(|(_, node)| node.sample)
        .min()
        .unwrap_or(options.samples);
    (0..task_list.nodes.len()).find(|&index| {
        task_list.nodes[index].sample < first_unfinished_sample + options.threads
            && ready_input_edges(task_list, states, pending_edges, index).is_some()
    })
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
    let outputs = match node.operation {
        NodeOperation::Calculate {
            operation,
            sampled_parameters,
        } => {
            let sampled = sampled_parameters
                .map(|parameters| parameters.operation(&node.name, node.sample, seed))
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

fn emit_node_results(
    node: &TaskNode<'_>,
    outputs: &NodeOutputs,
    options: ExecutionOptions,
    on_node_result: &mut impl FnMut(NodeResult) -> Result<()>,
) -> Result<()> {
    for (port, layer) in &node.result_layers {
        if matches!(node.operation, NodeOperation::Streaming { .. }) {
            on_node_result(NodeResult {
                sample: None,
                layer: layer.clone(),
                outputs: outputs.clone(),
            })?;
        } else {
            let value = outputs
                .get(port)
                .ok_or_else(|| format!("missing result port {port}"))?;
            let samples = if node.is_shared {
                0..options.samples
            } else {
                node.sample..node.sample + 1
            };
            for sample in samples {
                on_node_result(NodeResult {
                    sample: Some(sample),
                    layer: layer.clone(),
                    outputs: BTreeMap::from([("data".into(), value.clone())]),
                })?;
            }
        }
    }
    Ok(())
}

fn finish_node_run(
    task_list: &NodeTaskList<'_>,
    states: &mut [NodeExecutionState],
    pending_edges: &mut [bool],
    node_index: usize,
    update: NodeUpdate,
    options: ExecutionOptions,
    on_node_result: &mut impl FnMut(NodeResult) -> Result<()>,
) -> Result<()> {
    states[node_index].outputs = update.outputs;
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
        );
    }
    release_unused_outputs(task_list, states, pending_edges, node_index);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn execute_with_workers(
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
    fn graph_is_sorted_and_has_one_streaming_node_per_requested_layer() {
        let graph = graph();
        let task_list = build_node_task_list(
            &graph,
            HashMap::from([("data".into(), PipelineData::raster(vec![1.0]))]),
            options(),
            None,
        )
        .unwrap();
        assert_eq!(
            task_list
                .nodes
                .iter()
                .filter(|node| matches!(node.operation, NodeOperation::Streaming { .. }))
                .count(),
            1
        );
        for (edge_index, edge) in task_list.edges.iter().enumerate() {
            assert!(edge.source_node < edge.destination_node);
            assert!(task_list.nodes[edge.source_node]
                .output_edges
                .contains(&edge_index));
            assert!(task_list.nodes[edge.destination_node]
                .input_edges
                .contains(&edge_index));
        }
    }

    #[test]
    fn streaming_update_runs_before_other_samples_and_releases_its_input() {
        let graph = graph();
        let mut task_list = build_node_task_list(
            &graph,
            HashMap::from([("data".into(), PipelineData::raster(vec![1.0]))]),
            options(),
            None,
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
        execute_node_task_list(task_list, grid(), options(), |result| {
            assert_eq!(result.outputs["sum"].values()?, &[6.0]);
            count += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(count, 1);
    }
}
