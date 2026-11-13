mod graph;
mod interval;
mod runtime;
mod schema;

pub use graph::Graph;
pub use runtime::{Field, Grid, NodeOutputs};
pub use schema::*;

type Result<T> = std::result::Result<T, String>;

#[cfg(test)]
mod tests;
