use super::{
    builder::resolve_model_connection,
    graph::{NodeOperation, NodeTaskList},
};
use crate::pipeline::{operations::Operation, CurrentSolver, PipelineData, PipelineGraph};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap},
};

/// Add current calculations directly to the task graph; leave the model unchanged.
pub(super) fn add_current_nodes(
    task_list: &mut NodeTaskList<'_>,
    graph: &PipelineGraph,
    current_solver: &CurrentSolver<'_>,
    samples: usize,
    varying_nodes: &mut BTreeSet<String>,
    node_indices: &mut HashMap<(usize, String), usize>,
) {
    let source_node = task_list.add_node(
        "#current_source",
        0,
        true,
        NodeOperation::Input,
        BTreeMap::from([(
            "data".into(),
            PipelineData::raster(current_solver.source.to_vec()),
        )]),
    );
    let ground_node = task_list.add_node(
        "#current_ground",
        0,
        true,
        NodeOperation::Input,
        BTreeMap::from([(
            "data".into(),
            PipelineData::raster(current_solver.ground.to_vec()),
        )]),
    );
    let (first_resistance_node, _) =
        resolve_model_connection(&graph.output, 0, varying_nodes, node_indices);
    let is_shared = task_list.nodes[first_resistance_node].is_shared;
    if !is_shared {
        varying_nodes.insert("#current".into());
    }
    let calculation_count = if is_shared { 1 } else { samples };
    for sample in 0..calculation_count {
        let current_node = task_list.add_node(
            "#current",
            sample,
            is_shared,
            NodeOperation::Calculate {
                operation: Cow::Owned(Operation::Current {
                    current_solver_settings: current_solver.settings,
                }),
                sampled_parameters: None,
            },
            BTreeMap::new(),
        );
        let (resistance_node, port) =
            resolve_model_connection(&graph.output, sample, varying_nodes, node_indices);
        task_list.add_edge(resistance_node, current_node, port, "resistance".into());
        task_list.add_edge(source_node, current_node, "data".into(), "source".into());
        task_list.add_edge(ground_node, current_node, "data".into(), "ground".into());
        node_indices.insert((sample, "#current".into()), current_node);
    }
}
