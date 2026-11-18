use super::Result;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug)]
pub enum PipelineData {
    Raster(Arc<Vec<f64>>),
    Points(Arc<Vec<f64>>),
}
impl PipelineData {
    pub fn raster(values: Vec<f64>) -> Self {
        Self::Raster(Arc::new(values))
    }
    pub(super) fn values(&self) -> Result<&[f64]> {
        match self {
            Self::Raster(values) => Ok(values),
            _ => Err("expected a raster input".into()),
        }
    }
}

pub type NodeOutputs = BTreeMap<String, PipelineData>;

/// Identifies the map produced by a node.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OutputLayer {
    Resistance,
    Current,
    Intermediate {
        node_name: String,
        port_name: String,
    },
}

/// A completed layer, or its completed mean and variance when sample is None.
#[derive(Clone, Debug)]
pub struct NodeResult {
    pub sample: Option<usize>,
    pub layer: OutputLayer,
    pub outputs: NodeOutputs,
}

/// Controls the shared executor for single runs and parameter samples.
#[derive(Clone, Copy, Debug)]
pub struct ExecutionOptions {
    pub samples: usize,
    /// Worker count. The standard Wasm build supports one worker.
    pub threads: usize,
    pub seed: u64,
    pub all_layers: bool,
    /// Accumulate means and sample variances. Requires at least two samples.
    /// Disabled for an ordinary run, so no statistics arrays are allocated.
    pub statistics: bool,
}
