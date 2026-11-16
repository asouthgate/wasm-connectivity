use lm_connect::pipeline::{files::input_names, ExecutionOptions};
use std::{collections::HashMap, env, fs, io, path::PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "Usage: lmc-resistance --model model.json --<input-name> FILE ... --output-dir results/ [--all-layers] [--samples N --seed N --save-samples] [--current-source FILE --current-ground FILE] [-v|--verbose]

Raster inputs must be single-band GeoTIFFs with north-up square pixels and a projected CRS in metres.
An explicit model grid may crop or extend their bounds; CRS, cellsize and cell alignment must match.
Input types define their data requirements: vector_mask uses geometry; occluders and raycasters require height; vector_resistance requires resistance.
GeoJSON coordinates must already use the grid CRS. GeoPackages must contain one vector layer in that CRS.
The model defines input names and types. Each input requires a matching --<name> FILE flag.
File paths are relative to the working directory.
--all-layers also writes each named raster layer and surface output.
The final output is total_resistance.tif. Outputs are georeferenced GeoTIFF layers.
--samples runs complete parameter draws, writing means and sample variances (N-1).
--save-samples also writes each completed sample under samples/.
Current inputs append the existing f64 current solver to each sample before accumulation.";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

struct Args {
    pipeline_json: String,
    paths: HashMap<String, PathBuf>,
    verbose: bool,
    execution_options: ExecutionOptions,
    save_samples: bool,
    // Resistance comes from the pipeline; source and ground are read from these files.
    current_source_path: Option<PathBuf>,
    current_ground_path: Option<PathBuf>,
}

fn parse_args(arguments: &[String]) -> Result<Args> {
    let mut paths = HashMap::new();
    let mut verbose = false;
    let mut all_layers = false;
    let mut save_samples = false;
    let mut i = 0;
    while i < arguments.len() {
        let flag = &arguments[i];
        match flag.as_str() {
            "--save-samples" => {
                if save_samples {
                    return Err(invalid("duplicate --save-samples").into());
                }
                save_samples = true;
            }
            "--all-layers" => {
                if all_layers {
                    return Err(invalid("duplicate --all-layers").into());
                }
                all_layers = true;
            }
            "-v" | "--verbose" => {
                verbose = true;
            }
            _ if flag.starts_with("--") && flag.len() > 2 => {
                let value = arguments
                    .get(i + 1)
                    .filter(|s| !s.starts_with('-'))
                    .ok_or_else(|| invalid(format!("missing value for {flag}")))?;
                if paths.insert(flag.clone(), PathBuf::from(value)).is_some() {
                    return Err(invalid(format!("duplicate argument: {flag}")).into());
                }
                i += 1;
            }
            _ => return Err(invalid(format!("unknown argument: {flag}")).into()),
        }
        i += 1;
    }
    for name in ["--model", "--output-dir"] {
        if !paths.contains_key(name) {
            return Err(invalid(format!("{name} is required")).into());
        }
    }
    let samples = paths
        .remove("--samples")
        .map(|value| {
            value
                .to_str()
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|&n| n >= 2)
                .ok_or_else(|| invalid("--samples must be an integer of at least two"))
        })
        .transpose()?;
    let seed = paths
        .remove("--seed")
        .map(|value| {
            value
                .to_str()
                .and_then(|v| v.parse::<u64>().ok())
                .ok_or_else(|| invalid("--seed must be an unsigned integer"))
        })
        .transpose()?;
    let current_source_path = paths.remove("--current-source");
    let current_ground_path = paths.remove("--current-ground");
    if current_source_path.is_some() != current_ground_path.is_some() {
        return Err(
            invalid("--current-source and --current-ground must be supplied together").into(),
        );
    }
    if samples.is_none() && (seed.is_some() || save_samples || current_source_path.is_some()) {
        return Err(invalid("--seed, --save-samples, and current inputs require --samples").into());
    }
    let model_path = &paths["--model"];
    let pipeline_json = fs::read_to_string(model_path)?;
    let names = input_names(&pipeline_json)
        .map_err(|e| invalid(format!("{}: {e}", model_path.display())))?;
    for name in &names {
        if matches!(
            name.as_str(),
            "model"
                | "output-dir"
                | "all-layers"
                | "verbose"
                | "help"
                | "samples"
                | "seed"
                | "save-samples"
                | "current-source"
                | "current-ground"
        ) {
            return Err(invalid(format!(
                "input name {name} conflicts with a command-line option"
            ))
            .into());
        }
    }
    for flag in paths
        .keys()
        .filter(|flag| flag.as_str() != "--model" && flag.as_str() != "--output-dir")
    {
        if !names.iter().any(|name| name == &flag[2..]) {
            return Err(invalid(format!("unknown input argument: {flag}")).into());
        }
    }
    for name in &names {
        if !paths.contains_key(&format!("--{name}")) {
            return Err(invalid(format!("--{name} is required by model input {name}")).into());
        }
    }
    Ok(Args {
        pipeline_json,
        paths,
        verbose,
        execution_options: ExecutionOptions {
            samples: samples.unwrap_or(1),
            seed: seed.unwrap_or(0),
            all_layers,
            statistics: samples.is_some(),
        },
        save_samples,
        current_source_path,
        current_ground_path,
    })
}

fn execute(args: &Args) -> Result<()> {
    if args.verbose {
        eprintln!("Evaluating pipeline");
    }
    let paths = args
        .paths
        .iter()
        .filter(|(name, _)| name.as_str() != "--model" && name.as_str() != "--output-dir")
        .map(|(name, path)| (name[2..].to_string(), path.clone()))
        .collect();
    let warnings = lm_connect::pipeline::files::execute_pipeline_from_files(
        &args.pipeline_json,
        &paths,
        &args.paths["--output-dir"],
        args.execution_options,
        args.save_samples,
        args.current_source_path.as_deref(),
        args.current_ground_path.as_deref(),
    )?;
    for warning in warnings {
        eprintln!("Warning: {warning}");
    }
    Ok(())
}

fn main() {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments.len() == 1 && matches!(arguments[0].as_str(), "--help" | "-h") {
        println!("{USAGE}");
        return;
    }
    let args = match parse_args(&arguments) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    if let Err(e) = execute(&args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = env::temp_dir().join(format!(
                "lmc-resistance-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn input_flags_are_inferred_from_the_model() {
        let dir = TestDir::new();
        let path = dir.0.join("model.json");
        let mut model = json!({
          "name": "custom",
          "inputs": {
            "canopy_height": {
              "type": "raster_ranked"
            }
          },
          "nodes": {},
          "output": "inputs.canopy_height"
        });
        fs::write(&path, model.to_string()).unwrap();
        let base = vec![
            "--output-dir".to_string(),
            "results".into(),
            "--model".into(),
            path.to_str().unwrap().into(),
        ];
        assert!(parse_args(&base)
            .err()
            .unwrap()
            .to_string()
            .contains("--canopy_height is required"));
        let mut valid = vec!["--canopy_height".into(), "height.tif".into()];
        valid.extend(base.clone());
        assert!(parse_args(&valid).is_ok());
        for extra in [
            vec!["--dsm", "dsm.tif"],
            vec!["--canopy_height", "again.tif"],
            vec!["--all-layers", "--all-layers"],
            vec!["--config", "old.json"],
        ] {
            let mut arguments = valid.clone();
            arguments.extend(extra.into_iter().map(String::from));
            assert!(parse_args(&arguments).is_err());
        }
        assert!(parse_args(&["--model".into()]).is_err());
        model["inputs"] = json!({
          "output-dir": {
            "type": "raster_ranked"
          }
        });
        model["output"] = json!("inputs.output-dir");
        fs::write(&path, model.to_string()).unwrap();
        assert!(parse_args(&base)
            .err()
            .unwrap()
            .to_string()
            .contains("conflicts"));
    }
}
