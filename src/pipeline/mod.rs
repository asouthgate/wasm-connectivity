//! Run pipeline JSON with [`execute`].

#[cfg(all(feature = "bin", not(target_arch = "wasm32")))]
pub mod files;

mod model;
mod operations;
mod task_graph;
mod types;

pub use crate::current::{CurrentSolver, CurrentSolverSettings};
pub use model::schema::Grid;
use model::schema::PipelineDataReference;
pub(crate) use model::PipelineGraph;
pub use types::{ExecutionOptions, NodeOutputs, NodeResult, OutputLayer, PipelineData};

use std::{borrow::Cow, collections::HashMap};

type Result<T> = std::result::Result<T, String>;

pub(crate) fn execute_graph(
    graph: &PipelineGraph,
    inputs: HashMap<String, PipelineData>,
    grid: Grid,
    options: ExecutionOptions,
    current_solver: Option<&CurrentSolver<'_>>,
    on_node_result: impl FnMut(NodeResult) -> Result<()>,
) -> Result<()> {
    task_graph::validate_execution_options(graph, options)?;
    let mut inputs = graph.prepare_inputs(inputs, grid)?;
    let mut graph = Cow::Borrowed(graph);
    if let Some(solver) = current_solver {
        solver.validate(grid.nrows * grid.ncols)?;
        graph.to_mut().add_current(&mut inputs, solver);
    }
    task_graph::execute(&graph, inputs, grid, options, on_node_result)
}

/// Execute pipeline JSON and receive layers and summaries as they finish.
pub fn execute(
    pipeline_json: &str,
    inputs: HashMap<String, PipelineData>,
    grid: Grid,
    options: ExecutionOptions,
    current_solver: Option<&crate::pipeline::CurrentSolver<'_>>,
    on_node_result: impl FnMut(NodeResult) -> Result<()>,
) -> Result<()> {
    let graph = PipelineGraph::from_json(pipeline_json)?;
    execute_graph(
        &graph,
        inputs,
        grid,
        options,
        current_solver,
        on_node_result,
    )
}
