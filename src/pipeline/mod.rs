//! Run pipeline JSON with [`execute`].

#[cfg(all(feature = "bin", not(target_arch = "wasm32")))]
pub mod files;

mod current_solver;
pub use current_solver::CurrentSolver;

mod execution;
mod execution_state;
mod graph;
mod inputs;
mod interval;
mod operations;
mod sampling;
mod schema;
pub use sampling::{ResultSummaryStatistics, SampleResult, SamplingResult};

pub use execution::{ExecutionOptions, NodeOutputs, PipelineData};
pub(crate) use graph::PipelineGraph;
pub use schema::Grid;
use schema::Input;
use schema::{InputEdges, Operation, Reference};

type Result<T> = std::result::Result<T, String>;

#[cfg(test)]
mod tests;

/// Execute pipeline JSON with named input data and export each completed sample.
pub fn execute(
    pipeline_json: &str,
    inputs: std::collections::HashMap<String, PipelineData>,
    grid: Grid,
    options: ExecutionOptions,
    current_solver: Option<&crate::pipeline::CurrentSolver<'_>>,
    export_callback: impl FnMut(&SampleResult) -> Result<()>,
) -> Result<Option<SamplingResult>> {
    PipelineGraph::from_json(pipeline_json)?.execute(
        inputs,
        grid,
        options,
        current_solver,
        export_callback,
    )
}
