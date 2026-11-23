mod inputs;
mod outputs;

use super::{ExecutionOptions, OutputLayer, PipelineGraph};
pub use inputs::input_names;
use inputs::{load_current_inputs, load_inputs, model_grid};
use outputs::write_node_result;
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// Load pipeline inputs from files, execute, and write maps or summary statistics.
pub fn execute_pipeline_from_files(
    pipeline_json: &str,
    paths: &HashMap<String, PathBuf>,
    output_dir: &Path,
    options: ExecutionOptions,
    save_samples: bool,
    current_source_path: Option<&Path>,
    current_ground_path: Option<&Path>,
    report_sample_completion: impl FnMut(usize) -> Result<()>,
) -> Result<Vec<String>> {
    execute_pipeline_from_files_with_trace(
        pipeline_json,
        paths,
        output_dir,
        options,
        save_samples,
        current_source_path,
        current_ground_path,
        false,
        report_sample_completion,
    )
}

pub fn execute_pipeline_from_files_with_trace(
    pipeline_json: &str,
    paths: &HashMap<String, PathBuf>,
    output_dir: &Path,
    options: ExecutionOptions,
    save_samples: bool,
    current_source_path: Option<&Path>,
    current_ground_path: Option<&Path>,
    verbose: bool,
    report_sample_completion: impl FnMut(usize) -> Result<()>,
) -> Result<Vec<String>> {
    let graph = PipelineGraph::from_json(pipeline_json).map_err(invalid)?;
    execute_pipeline_graph_from_files(
        &graph,
        paths,
        output_dir,
        options,
        save_samples,
        current_source_path,
        current_ground_path,
        verbose,
        report_sample_completion,
    )
}

fn execute_pipeline_graph_from_files(
    graph: &PipelineGraph,
    paths: &HashMap<String, PathBuf>,
    output_dir: &Path,
    mut options: ExecutionOptions,
    save_samples: bool,
    current_source_path: Option<&Path>,
    current_ground_path: Option<&Path>,
    verbose: bool,
    mut report_sample_completion: impl FnMut(usize) -> Result<()>,
) -> Result<Vec<String>> {
    options.statistics = options.samples > 1;
    options.all_layers |= save_samples;
    let loaded = load_inputs(graph, paths)?;
    let current_source_and_ground =
        load_current_inputs(&loaded.grid, current_source_path, current_ground_path)?;
    let current_solver = current_source_and_ground.as_ref().map(|(source, ground)| {
        crate::pipeline::CurrentSolver::new(source, ground, Default::default())
    });
    let mut completed_samples = 0;
    super::execute_graph_with_trace(
        graph,
        loaded.fields,
        model_grid(&loaded.grid),
        options,
        current_solver.as_ref(),
        verbose,
        true,
        |result| {
            if result.sample.is_none() || options.samples == 1 || save_samples {
                write_node_result(output_dir, &loaded.grid, options.samples, &result)
                    .map_err(|error| error.to_string())?;
            }
            let is_final_map = if current_solver.is_some() {
                result.layer == OutputLayer::Current
            } else {
                result.layer == OutputLayer::Resistance
            };
            if result.sample.is_some() && is_final_map {
                completed_samples += 1;
                report_sample_completion(completed_samples).map_err(|error| error.to_string())?;
            }
            Ok(())
        },
    )
    .map_err(invalid)?;
    Ok(loaded.warnings)
}

#[cfg(test)]
mod tests;
