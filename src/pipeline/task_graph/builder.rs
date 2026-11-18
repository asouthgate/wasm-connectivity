use super::graph::{sort_nodes_topologically, NodeOperation, NodeTaskList};
use crate::pipeline::{
    model::schema::{InputEdges, Node},
    ExecutionOptions, PipelineData, PipelineDataReference, PipelineGraph, Result,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Create nodes whose results stay the same across samples once, and create the
/// other nodes for each sample. Connect the nodes and sort them by dependency.
pub(in crate::pipeline) fn build_node_task_list<'a>(
    graph: &'a PipelineGraph,
    inputs: HashMap<String, PipelineData>,
    options: ExecutionOptions,
) -> Result<NodeTaskList<'a>> {
    let mut task_list = NodeTaskList {
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let mut node_indices = HashMap::new();
    let varying_nodes = sample_dependent_nodes(graph);
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
    task_list.connect_results_and_statistics(graph, options, &varying_nodes, &node_indices);
    sort_nodes_topologically(task_list)
}

impl<'a> NodeTaskList<'a> {
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
}

fn sample_dependent_nodes(graph: &PipelineGraph) -> BTreeSet<String> {
    let mut varying_nodes = BTreeSet::new();
    for name in &graph.order {
        let node = &graph.nodes[name];
        if node.sampling.is_some() || node.inputs.values().flat_map(InputEdges::references)
                .any(|r| matches!(r, PipelineDataReference::Node { from_node, .. } if varying_nodes.contains(from_node))) {
                varying_nodes.insert(name.clone());
            }
    }
    varying_nodes
}

pub(super) fn resolve_model_connection(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{operations::Operation, CurrentSolver};
    use serde_json::json;
    fn options() -> ExecutionOptions {
        ExecutionOptions {
            samples: 3,
            threads: 2,
            seed: 0,
            all_layers: false,
            statistics: true,
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
    fn model_current_node_expands_like_any_sample_dependent_node() {
        let mut graph = graph();
        let source = [1.0];
        let ground = [1.0];
        let current_solver = CurrentSolver::new(&source, &ground, Default::default());
        let mut inputs = HashMap::from([("data".into(), PipelineData::raster(vec![1.0]))]);
        graph.add_current(&mut inputs, &current_solver);
        assert!(matches!(
            graph.nodes["#current"].operation,
            Operation::Current { .. }
        ));
        assert!(graph
            .current_output
            .as_ref()
            .is_some_and(|output| output.selects_node("#current", "data")));
        let task_list = build_node_task_list(&graph, inputs, options()).unwrap();
        let current_nodes: Vec<_> = task_list
            .nodes
            .iter()
            .filter(|node| node.name == "#current")
            .collect();
        assert_eq!(current_nodes.len(), 3);
        for node in current_nodes {
            assert_eq!(node.input_edges.len(), 3);
            assert!(matches!(
                &node.operation,
                NodeOperation::Calculate {
                    operation: Operation::Current { .. },
                    ..
                }
            ));
        }
    }
}
