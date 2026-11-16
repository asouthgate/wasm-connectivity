#[cfg(all(feature = "bin", not(target_arch = "wasm32")))]
pub mod files;

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

impl Model {
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|error| format!("Invalid model JSON: {error}"))
    }
}
