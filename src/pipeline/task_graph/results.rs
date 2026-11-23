use super::{
    builder::resolve_model_connection,
    graph::{NodeOperation, NodeTaskList, TaskNode},
    stats::StatisticsAccumulator,
};
use crate::pipeline::{
    ExecutionOptions, NodeOutputs, NodeResult, OutputLayer, PipelineDataReference, PipelineGraph,
    Result,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

impl<'a> NodeTaskList<'a> {
    pub(super) fn connect_results_and_statistics(
        &mut self,
        graph: &PipelineGraph,
        options: ExecutionOptions,
        varying_nodes: &BTreeSet<String>,
        node_indices: &HashMap<(usize, String), usize>,
        fixed_intermediates_once: bool,
    ) {
        let mut layers = vec![(OutputLayer::Resistance, graph.output.clone())];
        if let Some(reference) = &graph.current_output {
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
            if fixed_intermediates_once && matches!(layer, OutputLayer::Intermediate { .. }) {
                let (source, port) =
                    resolve_model_connection(&reference, 0, varying_nodes, node_indices);
                if self.nodes[source].is_shared {
                    self.nodes[source].emit_fixed_intermediates_once = true;
                    self.nodes[source].result_layers.insert(port, layer);
                    continue;
                }
            }
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

pub(super) fn emit_node_results(
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
            if node.emit_fixed_intermediates_once
                && matches!(layer, OutputLayer::Intermediate { .. })
            {
                on_node_result(NodeResult {
                    sample: None,
                    layer: layer.clone(),
                    outputs: BTreeMap::from([("data".into(), value.clone())]),
                })?;
                continue;
            }
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
