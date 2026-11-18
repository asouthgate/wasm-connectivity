use crate::pipeline::{ExecutionOptions, Grid, NodeResult, PipelineData, PipelineGraph, Result};
use std::collections::HashMap;

fn validate_execution_options(graph: &PipelineGraph, options: ExecutionOptions) -> Result<()> {
    if options.threads == 0 {
        return Err("execution requires at least one thread".into());
    }
    #[cfg(target_arch = "wasm32")]
    if options.threads != 1 {
        return Err("Wasm execution requires one thread".into());
    }
    if options.samples == 0 {
        return Err("execution requires at least one sample".into());
    }
    if options.statistics && options.samples < 2 {
        return Err("sample variance requires at least two samples".into());
    }
    for (name, node) in &graph.nodes {
        if let Some(sampling) = &node.sampling {
            sampling
                .validate_count(options.samples)
                .map_err(|e| format!("node {name}: {e}"))?;
        }
    }
    Ok(())
}

/// Execute the pipeline.
///
/// - `inputs`: Named external rasters and point arrays; vectors must be rasterized first.
/// - `grid`: Array dimensions and cell size in metres.
/// - `options`: Sample count, seed, intermediate outputs, and statistics settings.
/// - `current_solver`: Optional solver applied to each resistance map.
/// - `on_node_result`: Receives each completed layer or summary without waiting for other layers.
///
/// Errors stop execution.
pub(crate) fn execute(
    graph: &PipelineGraph,
    inputs: HashMap<String, PipelineData>,
    grid: Grid,
    options: ExecutionOptions,
    current_solver: Option<&crate::pipeline::CurrentSolver<'_>>,
    on_node_result: impl FnMut(NodeResult) -> Result<()>,
) -> Result<()> {
    validate_execution_options(graph, options)?;
    let inputs = graph.prepare_inputs(inputs, grid)?;
    if let Some(solver) = current_solver {
        solver.validate(grid.nrows * grid.ncols)?;
    }
    let task_list = super::builder::build_node_task_list(graph, inputs, options, current_solver)?;
    super::scheduler::execute_node_task_list(task_list, grid, options, on_node_result)
}
