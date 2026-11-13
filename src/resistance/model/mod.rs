mod execution;
mod graph;
mod interval;
mod schema;

pub use execution::{Grid, NodeOutputs, PipelineData};
pub use graph::Graph;
pub use schema::*;

type Result<T> = std::result::Result<T, String>;

#[cfg(test)]
mod tests;
