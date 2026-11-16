use lm_connect::resistance::model::Model;
use std::{collections::HashMap, env, fs, io, path::PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "Usage: lmc-resistance --model model.json --<input-name> FILE ... --output-dir results/ [--all-layers] [-v|--verbose]

Raster inputs must be single-band GeoTIFFs with north-up square pixels and a projected CRS in metres.
An explicit model grid may crop or extend their bounds; CRS, cellsize and cell alignment must match.
Input types define their data requirements: vector_mask uses geometry; occluders and raycasters require height; vector_resistance requires resistance.
GeoJSON coordinates must already use the grid CRS. GeoPackages must contain one vector layer in that CRS.
The model defines input names and types. Each input requires a matching --<name> FILE flag.
File paths are relative to the working directory.
--all-layers also writes each named raster layer and surface output.
The final output is total_resistance.tif. Outputs are georeferenced GeoTIFF layers.";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

struct Args {
    model: Model,
    paths: HashMap<String, PathBuf>,
    all_layers: bool,
    verbose: bool,
}

fn parse_args(arguments: &[String]) -> Result<Args> {
    let mut paths = HashMap::new();
    let mut verbose = false;
    let mut all_layers = false;
    let mut i = 0;
    while i < arguments.len() {
        let flag = &arguments[i];
        match flag.as_str() {
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
    let model_path = &paths["--model"];
    let model = Model::from_json(&fs::read_to_string(model_path)?)
        .map_err(|e| invalid(format!("{}: {e}", model_path.display())))?;
    for name in model.inputs.keys() {
        if matches!(
            name.as_str(),
            "model" | "output-dir" | "all-layers" | "verbose" | "help"
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
        if !model.inputs.contains_key(&flag[2..]) {
            return Err(invalid(format!("unknown input argument: {flag}")).into());
        }
    }
    for name in model.inputs.keys() {
        if !paths.contains_key(&format!("--{name}")) {
            return Err(invalid(format!("--{name} is required by model input {name}")).into());
        }
    }
    Ok(Args {
        model,
        paths,
        all_layers,
        verbose,
    })
}

fn execute(args: &Args) -> Result<()> {
    if args.verbose {
        eprintln!("Evaluating model {}", args.model.name);
    }
    let paths = args
        .model
        .inputs
        .keys()
        .map(|name| (name.clone(), args.paths[&format!("--{name}")].clone()))
        .collect();
    let result = lm_connect::resistance::model::files::run_files(&args.model, &paths)?;
    for warning in &result.warnings {
        eprintln!("Warning: {warning}");
    }
    result.write(&args.paths["--output-dir"], args.all_layers)
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
        fs::write(&path, model.to_string()).unwrap();
        assert!(parse_args(&base)
            .err()
            .unwrap()
            .to_string()
            .contains("conflicts"));
    }
}
