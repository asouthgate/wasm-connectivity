use crate::pipeline::{
    model::parameters::SampledParameters, operations::Operation, NodeOutputs, OutputLayer,
    PipelineData, Result,
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, VecDeque},
};

pub(super) trait StreamingNode: Send {
    fn update(&mut self, inputs: &BTreeMap<String, Vec<PipelineData>>) -> Result<()>;
    fn finish(self: Box<Self>) -> Result<NodeOutputs>;
}

#[derive(Clone)]
pub(super) enum NodeOperation<'a> {
    Input,
    Calculate {
        operation: Cow<'a, Operation>,
        sampled_parameters: Option<&'a SampledParameters>,
    },
    Streaming {
        create: fn() -> Box<dyn StreamingNode>,
    },
}

pub(super) struct TaskNode<'a> {
    pub(super) name: String,
    pub(super) sample: usize,
    pub(super) is_shared: bool,
    pub(super) operation: NodeOperation<'a>,
    pub(super) input_edges: Vec<usize>,
    pub(super) output_edges: Vec<usize>,
    pub(super) initial_outputs: NodeOutputs,
    pub(super) result_layers: BTreeMap<String, OutputLayer>,
}

pub(super) struct NodeEdge {
    pub(super) source_node: usize,
    pub(super) destination_node: usize,
    pub(super) output_port: String,
    pub(super) input_port: String,
}

pub(in crate::pipeline) struct NodeTaskList<'a> {
    pub(super) nodes: Vec<TaskNode<'a>>,
    pub(super) edges: Vec<NodeEdge>,
}

impl<'a> NodeTaskList<'a> {
    pub(super) fn add_node(
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

    pub(super) fn add_edge(
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
}

pub(super) fn sort_nodes_topologically(
    mut task_list: NodeTaskList<'_>,
) -> Result<NodeTaskList<'_>> {
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
