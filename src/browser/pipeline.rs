use crate::{pipeline, solve};
use super::output;
use wasm_bindgen::prelude::*;

/// Named, already-prepared input arrays for the model executor.
#[wasm_bindgen]
#[derive(Default)]
pub struct NamedInputArrays {
    values: std::collections::HashMap<String, Vec<f64>>,
}

#[wasm_bindgen]
impl NamedInputArrays {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self { Self::default() }

    pub fn insert(&mut self, name: String, values: Vec<f64>) -> Result<(), String> {
        if self.values.contains_key(&name) {
            return Err(format!("duplicate model input: {name}"));
        }
        self.values.insert(name, values);
        Ok(())
    }
}

#[derive(serde::Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct PipelineExecutionJsonOptions {
    samples: usize,
    seed: u64,
    all_layers: bool,
    current_solver_options: Option<CurrentSolverJsonOptions>,
}

impl Default for PipelineExecutionJsonOptions {
    fn default() -> Self {
        Self { samples: 1, seed: 0, all_layers: true, current_solver_options: None }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CurrentSolverJsonOptions {
    max_iter: Option<usize>,
    tol: Option<f64>,
    #[serde(default)]
    use_dirichlet_ground: bool,
}

/// Execute pipeline JSON; one sample returns maps, multiple samples return statistics.
#[wasm_bindgen]
pub fn run_resistance_pipeline_browser(
    model_json: String,
    inputs: &mut NamedInputArrays,
    nrows: usize,
    ncols: usize,
    cellsize: f64,
    options_json: String,
    source: Vec<f64>,
    ground: Vec<f64>,
) -> Result<BrowserResistanceOutput, String> {
    use pipeline::{CurrentSolver, ExecutionOptions, Grid, PipelineData, PipelineGraph};
    use std::collections::BTreeMap;

    let graph = PipelineGraph::from_json(&model_json)?;
    let options: PipelineExecutionJsonOptions = serde_json::from_str(&options_json)
        .map_err(|e| e.to_string())?;
    let current_solver = options.current_solver_options.map(|options| {
        let mut solver = CurrentSolver::new(&source, &ground, Default::default());
        if let Some(max_iter) = options.max_iter { solver.settings.max_iter = max_iter; }
        if let Some(tol) = options.tol { solver.settings.tolerance = tol; }
        if options.use_dirichlet_ground { solver.settings.ground_mode = solve::GroundMode::Dirichlet; }
        solver
    });
    let mut output = BrowserResistanceOutput {
        names: Vec::new(), rasters: Vec::new(), nrows, ncols,
        samples: options.samples, has_current: current_solver.is_some(),
    };
    let statistics = options.samples > 1;
    let named_arrays = std::mem::take(&mut inputs.values);
    let pipeline_inputs = graph.convert_named_arrays_to_pipeline_inputs(named_arrays)?;
    let mut completed_layers = BTreeMap::new();
    crate::pipeline::execute_graph(
        &graph,
        pipeline_inputs,
        Grid { nrows, ncols, pixel_width: cellsize },
        ExecutionOptions {
            threads: 1,
            samples: options.samples, seed: options.seed,
            all_layers: options.all_layers, statistics,
        },
        current_solver.as_ref(),
        |result| {
            if result.sample.is_none() == statistics {
                completed_layers.insert(result.layer, result.outputs);
            }
            Ok(())
        },
    )?;
    for (layer, outputs) in completed_layers {
        if let pipeline::OutputLayer::Intermediate { node_name, port_name } = layer {
            output.names.push([node_name, port_name]);
        }
        let ports: &[&str] = if statistics { &["mean", "variance"] } else { &["data"] };
        for port in ports {
            let field = outputs.get(*port).ok_or_else(|| format!("missing output {port}"))?;
            if let PipelineData::Raster(values) = field { output.rasters.push(values.clone()); }
        }
    }
    Ok(output)
}

#[wasm_bindgen]
pub struct BrowserResistanceOutput {
    names: Vec<[String; 2]>,
    rasters: Vec<std::sync::Arc<Vec<f64>>>,
    nrows: usize,
    ncols: usize,
    samples: usize,
    has_current: bool,
}

#[wasm_bindgen]
impl BrowserResistanceOutput {
    #[wasm_bindgen(getter)]
    pub fn nrows(&self) -> usize { self.nrows }

    #[wasm_bindgen(getter)]
    pub fn ncols(&self) -> usize { self.ncols }

    #[wasm_bindgen(getter)]
    pub fn samples(&self) -> usize { self.samples }

    #[wasm_bindgen(getter)]
    pub fn has_current(&self) -> bool { self.has_current }

    pub fn layer_names(&self) -> String {
        serde_json::to_string(&self.names).expect("layer names are JSON strings")
    }

    /// Release the stored raster at its original precision.
    pub fn release_f64_raster(&mut self, index: usize) -> Result<Vec<f64>, String> {
        let raster = self.rasters.get_mut(index).ok_or("Unknown raster")?;
        Ok(std::sync::Arc::unwrap_or_clone(std::mem::take(raster)))
    }

    /// Convert a raster to f32 and release its stored reference; statistics use mean/variance pairs.
    pub fn release_and_convert_f64_raster_to_f32(&mut self, index: usize) -> Result<Vec<f32>, String> {
        output::release_and_convert_f64_raster_to_f32(&mut self.rasters, index)
    }
}
