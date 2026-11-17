use super::Result;
use lm_connect::pipeline::ExecutionOptions;
use std::{
    collections::HashMap,
    io::{self, Write},
    path::PathBuf,
};

pub(super) struct PipelineArguments {
    pub pipeline_json: String,
    pub input_paths: HashMap<String, PathBuf>,
    pub output_directory: PathBuf,
    pub execution_options: ExecutionOptions,
    pub save_samples: bool,
    pub current_source_path: Option<PathBuf>,
    pub current_ground_path: Option<PathBuf>,
    pub verbose: bool,
}

pub(super) fn execute(arguments: PipelineArguments) -> Result<()> {
    if arguments.verbose {
        eprintln!("Evaluating pipeline");
    }
    let mut sample_progress_displayed = false;
    let execution_result = lm_connect::pipeline::files::execute_pipeline_from_files(
        &arguments.pipeline_json,
        &arguments.input_paths,
        &arguments.output_directory,
        arguments.execution_options,
        arguments.save_samples,
        arguments.current_source_path.as_deref(),
        arguments.current_ground_path.as_deref(),
        |completed_samples| {
            if arguments.execution_options.samples > 1 {
                let mut stderr = io::stderr().lock();
                write!(
                    stderr,
                    "\rCompleted sample {completed_samples}/{}",
                    arguments.execution_options.samples
                )?;
                sample_progress_displayed = true;
                stderr.flush()?;
            }
            Ok(())
        },
    );
    if sample_progress_displayed {
        eprintln!();
    }
    let warnings = execution_result?;
    for warning in warnings {
        eprintln!("Warning: {warning}");
    }
    Ok(())
}
