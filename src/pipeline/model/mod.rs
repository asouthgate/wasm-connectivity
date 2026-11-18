//! Read and validate model JSON, input declarations, and sampled parameters.

mod graph;
mod inputs;
pub(super) mod parameters;
pub(super) mod schema;

pub(crate) use graph::PipelineGraph;

#[cfg(test)]
mod tests;
