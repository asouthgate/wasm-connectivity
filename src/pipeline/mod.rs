//! Run pipeline JSON with [`execute`].

#[cfg(all(feature = "bin", not(target_arch = "wasm32")))]
pub mod files;

mod model;
mod operations;
mod task_graph;
mod types;

pub use model::schema::Grid;
use model::schema::PipelineDataReference;
pub(crate) use model::PipelineGraph;
pub use crate::solve::{CurrentSolver, CurrentSolverSettings};
pub(crate) use task_graph::execute as execute_graph;
pub use types::{ExecutionOptions, NodeOutputs, NodeResult, OutputLayer, PipelineData};

type Result<T> = std::result::Result<T, String>;

/// Execute pipeline JSON and receive layers and summaries as they finish.
pub fn execute(
    pipeline_json: &str,
    inputs: std::collections::HashMap<String, PipelineData>,
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
