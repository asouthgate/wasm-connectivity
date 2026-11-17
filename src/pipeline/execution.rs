use super::{
    execution_state::{Evaluator, ExecutionState},
    graph::PipelineGraph,
    sampling::{ExecutionStatistics, SampleResult, SamplingResult},
    schema::*,
    Result,
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

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

/// Controls the shared executor for single runs and parameter samples.
#[derive(Clone, Copy, Debug)]
pub struct ExecutionOptions {
    pub samples: usize,
    pub seed: u64,
    pub all_layers: bool,
    /// Accumulate means and sample variances. Requires at least two samples.
    /// Disabled for an ordinary run, so no statistics arrays are allocated.
    pub statistics: bool,
}

impl PipelineGraph {
    /// Execute the pipeline.
    ///
    /// - `inputs`: Named external rasters and point arrays; vectors must be rasterized first.
    /// - `grid`: Array dimensions and cell size in metres.
    /// - `options`: Sample count, seed, intermediate outputs, and statistics settings.
    /// - `current_solver`: Optional solver applied to each resistance map.
    /// - `export_callback`: Receives each completed sample for collection or file export.
    ///
    /// Returns statistics if requested, otherwise `None`; errors stop execution.
    pub fn execute(
        &self,
        inputs: HashMap<String, PipelineData>,
        grid: Grid,
        options: ExecutionOptions,
        current_solver: Option<&crate::pipeline::CurrentSolver<'_>>,
        mut export_callback: impl FnMut(&SampleResult) -> Result<()>,
    ) -> Result<Option<SamplingResult>> {
        self.validate_execution_options(options)?;
        let inputs = self.prepare_inputs(inputs, grid)?;
        if let Some(solver) = current_solver {
            solver.validate(grid.nrows * grid.ncols)?;
        }
        let execution_state = self.run_constant_pipeline_nodes(inputs, grid, options.all_layers)?;
        let mut statistics = options
            .statistics
            .then(|| ExecutionStatistics::new(grid.nrows * grid.ncols, current_solver.is_some()));
        for index in 0..options.samples {
            let sample = execution_state.run_sample(index, options, current_solver)?;
            if let Some(statistics) = &mut statistics {
                statistics.update(&sample)?;
            }
            export_callback(&sample).map_err(|error| format!("sample {index}: {error}"))?;
        }
        Ok(statistics.map(ExecutionStatistics::finish))
    }

    fn validate_execution_options(&self, options: ExecutionOptions) -> Result<()> {
        if options.samples == 0 {
            return Err("execution requires at least one sample".into());
        }
        if options.statistics && options.samples < 2 {
            return Err("sample variance requires at least two samples".into());
        }
        for (name, node) in &self.nodes {
            if let Some(sampling) = &node.sampling {
                sampling
                    .validate_count(options.samples)
                    .map_err(|e| format!("node {name}: {e}"))?;
            }
        }
        Ok(())
    }

    fn sample_dependent_nodes(&self) -> BTreeSet<String> {
        let mut varying = BTreeSet::new();
        for name in &self.order {
            let node = &self.nodes[name];
            if node.sampling.is_some() || node.inputs.values().flat_map(InputEdges::references)
                .any(|r| matches!(r, Reference::Node { from_node, .. } if varying.contains(from_node))) {
                varying.insert(name.clone());
            }
        }
        varying
    }

    fn node_use_counts(&self) -> HashMap<String, usize> {
        let mut uses = HashMap::<String, usize>::new();
        for node in self.nodes.values() {
            for reference in node.inputs.values().flat_map(InputEdges::references) {
                if let Reference::Node { from_node, .. } = reference {
                    *uses.entry(from_node.clone()).or_default() += 1;
                }
            }
        }
        if let Reference::Node { from_node, .. } = &self.output {
            *uses.entry(from_node.clone()).or_default() += 1;
        }
        uses
    }

    /// Compute constant nodes once and retain results needed by sampled nodes.
    fn run_constant_pipeline_nodes(
        &self,
        inputs: HashMap<String, PipelineData>,
        grid: Grid,
        all_layers: bool,
    ) -> Result<ExecutionState<'_>> {
        let varying = self.sample_dependent_nodes();
        let mut fixed = Evaluator {
            graph: self,
            grid,
            inputs: inputs.clone(),
            outputs: HashMap::new(),
        };
        let mut remaining = self.node_use_counts();
        let mut fixed_layers = BTreeMap::new();
        for name in &self.order {
            if varying.contains(name) {
                continue;
            }
            let output = fixed.node(name)?;
            if all_layers && self.named_nodes.contains(name) {
                fixed_layers.insert(name.clone(), output);
            }
            fixed.release_unused_outputs(name, &mut remaining);
        }
        // Inputs used only by fixed nodes are no longer needed.
        fixed.inputs = self.sample_inputs(inputs, &varying);
        Ok(ExecutionState {
            base: fixed,
            varying,
            remaining,
            layers: fixed_layers,
        })
    }

    fn sample_inputs(
        &self,
        inputs: HashMap<String, PipelineData>,
        varying: &BTreeSet<String>,
    ) -> HashMap<String, PipelineData> {
        let mut needed_inputs = BTreeSet::new();
        for name in varying {
            for reference in self.nodes[name]
                .inputs
                .values()
                .flat_map(InputEdges::references)
            {
                if let Reference::Input { from_input } = reference {
                    needed_inputs.insert(from_input.clone());
                }
            }
        }
        if let Reference::Input { from_input } = &self.output {
            needed_inputs.insert(from_input.clone());
        }
        inputs
            .into_iter()
            .filter(|(name, _)| needed_inputs.contains(name))
            .collect()
    }

    pub(super) fn prepare_inputs(
        &self,
        mut inputs: HashMap<String, PipelineData>,
        grid: Grid,
    ) -> Result<HashMap<String, PipelineData>> {
        self.validate_grid(grid)?;
        for (name, spec) in &self.inputs {
            let field = inputs
                .get(name)
                .ok_or_else(|| format!("missing input: {name}"))?;
            validate_input(name, spec, field, grid)?;
        }
        if inputs.keys().any(|name| !self.inputs.contains_key(name)) {
            return Err("undeclared model input supplied".into());
        }
        for field in inputs.values_mut() {
            if let PipelineData::Points(points) = field {
                let retained = points
                    .chunks_exact(3)
                    .filter(|point| grid.contains_point(point))
                    .flatten()
                    .copied()
                    .collect();
                *points = Arc::new(retained);
            }
        }
        Ok(inputs)
    }
    fn validate_grid(&self, grid: Grid) -> Result<()> {
        if grid.nrows == 0
            || grid.ncols == 0
            || !grid.pixel_width.is_finite()
            || grid.pixel_width <= 0.0
            || grid.nrows.checked_mul(grid.ncols).is_none()
        {
            return Err("invalid model grid".into());
        }
        if let Some(spec) = &self.grid {
            let (rows, cols) = spec.dimensions()?;
            if (rows, cols) != (grid.nrows, grid.ncols)
                || (spec.cellsize - grid.pixel_width).abs() > spec.cellsize * 1e-9
            {
                return Err("model arrays do not match the explicit grid".into());
            }
        } else if !self.inputs.values().any(Input::is_raster) {
            return Err("model requires at least one raster input or an explicit grid".into());
        }
        Ok(())
    }
}
fn validate_input(name: &str, spec: &Input, field: &PipelineData, grid: Grid) -> Result<()> {
    match (spec, field) {
        (Input::RaycasterPointSource {}, PipelineData::Points(values)) if values.len() % 3 == 0 => {
            for point in values.chunks_exact(3) {
                if !point.iter().all(|v| v.is_finite()) || point[2] < 0.0 {
                    return Err(format!(
                        "invalid point coordinates or height in input {name}"
                    ));
                }
            }
        }
        (
            Input::RasterTopography {}
            | Input::RasterSurface {}
            | Input::RasterRanked {}
            | Input::VectorMask {}
            | Input::VectorOccluder {}
            | Input::VectorResistance {},
            PipelineData::Raster(values),
        ) if values.len() == grid.nrows * grid.ncols => {}
        _ => return Err(format!("wrong input type or dimensions: {name}")),
    }
    Ok(())
}
