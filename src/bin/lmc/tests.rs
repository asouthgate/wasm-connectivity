use super::{arguments, SubcommandArguments};
use clap::error::ErrorKind;
use serde_json::json;
use std::{ffi::OsString, fs, path::PathBuf};

struct TestModel {
    directory: PathBuf,
    path: String,
}

impl TestModel {
    fn new(input_name: &str) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "lmc-arguments-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("model.json");
        fs::write(
            &path,
            json!({
                "name": "custom", "inputs": {input_name: {"type": "raster_ranked"}},
                "nodes": {}, "output": format!("inputs.{input_name}")
            })
            .to_string(),
        )
        .unwrap();
        Self {
            directory,
            path: path.to_str().unwrap().to_string(),
        }
    }
}

impl Drop for TestModel {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn parse(values: &[&str]) -> Result<SubcommandArguments, clap::Error> {
    arguments::parse(values.iter().map(OsString::from).collect())
}

#[test]
fn input_flags_come_from_the_model_regardless_of_argument_order() {
    let model = TestModel::new("canopy_height");
    let subcommand_arguments = parse(&[
        "lmc",
        "resistance",
        "--canopy_height",
        "height.tif",
        "--model",
        &model.path,
        "--output-dir",
        "results",
    ])
    .unwrap();
    let SubcommandArguments::Resistance(arguments) = subcommand_arguments else {
        panic!("wrong command");
    };
    assert_eq!(
        arguments.input_paths["canopy_height"],
        PathBuf::from("height.tif")
    );
    assert_eq!(arguments.execution_options.samples, 1);
    assert!(arguments.current_source_path.is_none());

    let model_option = format!("--model={}", model.path);
    assert!(parse(&[
        "lmc",
        "resistance",
        &model_option,
        "--canopy_height",
        "height.tif",
        "--output-dir",
        "results"
    ])
    .is_ok());
    for extra in [
        vec!["--dsm", "unknown.tif"],
        vec!["--canopy_height", "again.tif"],
        vec!["--all-layers", "--all-layers"],
    ] {
        let mut values = vec![
            "lmc",
            "resistance",
            "--model",
            &model.path,
            "--canopy_height",
            "height.tif",
            "--output-dir",
            "results",
        ];
        values.extend(extra);
        assert!(parse(&values).is_err());
    }
    let error = parse(&[
        "lmc",
        "resistance",
        "--model",
        &model.path,
        "--output-dir",
        "results",
    ])
    .err()
    .unwrap();
    assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
    assert!(error.to_string().contains("--canopy_height"));
}

#[test]
fn help_lists_commands_and_model_specific_inputs() {
    let error = parse(&["lmc", "--help"]).err().unwrap();
    assert_eq!(error.kind(), ErrorKind::DisplayHelp);
    for command in ["resistance", "current", "full"] {
        assert!(error.to_string().contains(command));
    }
    let model = TestModel::new("habitat");
    let error = parse(&["lmc", "full", "--model", &model.path, "--help"])
        .err()
        .unwrap();
    assert_eq!(error.kind(), ErrorKind::DisplayHelp);
    assert!(error.to_string().contains("--habitat"));
    assert!(error.to_string().contains("--source"));
    assert!(error.to_string().contains("--ground"));
    assert_eq!(
        parse(&["lmc", "current", "--help"]).err().unwrap().kind(),
        ErrorKind::DisplayHelp
    );
}

#[test]
fn full_requires_both_current_inputs_and_supports_one_or_many_samples() {
    let model = TestModel::new("base");
    let mut values = vec![
        "lmc",
        "full",
        "--model",
        &model.path,
        "--base",
        "base.tif",
        "--output-dir",
        "results",
    ];
    assert_eq!(
        parse(&values).err().unwrap().kind(),
        ErrorKind::MissingRequiredArgument
    );
    values.extend(["--source", "source.tif", "--ground", "ground.tif"]);
    let SubcommandArguments::Full(arguments) = parse(&values).unwrap() else {
        panic!("wrong command");
    };
    assert_eq!(arguments.execution_options.samples, 1);
    assert!(!arguments.execution_options.statistics);
    assert_eq!(
        arguments.current_source_path.unwrap(),
        PathBuf::from("source.tif")
    );
    values.extend(["--samples", "5", "--seed", "42", "--save-samples"]);
    let SubcommandArguments::Full(arguments) = parse(&values).unwrap() else {
        panic!("wrong command");
    };
    assert_eq!(arguments.execution_options.samples, 5);
    assert_eq!(arguments.execution_options.seed, 42);
    assert!(arguments.execution_options.statistics);
    assert!(arguments.save_samples);
}

#[test]
fn reserved_input_names_are_checked_for_the_selected_mode() {
    let model = TestModel::new("output-dir");
    let error = parse(&["lmc", "resistance", "--model", &model.path, "--help"])
        .err()
        .unwrap();
    assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
    assert!(error.to_string().contains("conflicts"));
    let model = TestModel::new("source");
    assert!(parse(&[
        "lmc",
        "resistance",
        "--model",
        &model.path,
        "--source",
        "data.tif",
        "--output-dir",
        "results"
    ])
    .is_ok());
    assert_eq!(
        parse(&["lmc", "full", "--model", &model.path, "--help"])
            .err()
            .unwrap()
            .kind(),
        ErrorKind::ArgumentConflict
    );
}

#[test]
fn current_options_are_typed_and_preserve_solver_defaults() {
    for (precision, tolerance) in [
        ("f64", 1e-4),
        ("f32", f64::from(1e-3_f32)),
        ("mixed", f64::from(1e-3_f32)),
    ] {
        let SubcommandArguments::Current(arguments) = parse(&[
            "lmc",
            "current",
            "--resistance",
            "resistance.tif",
            "--source",
            "source.tif",
            "--ground",
            "ground.tif",
            "--precision",
            precision,
            "--l0postnu",
            "0",
        ])
        .unwrap() else {
            panic!("wrong command");
        };
        assert_eq!(arguments.tolerance, tolerance);
        assert_eq!(arguments.multigrid.l0postnu, 0);
        assert_eq!(arguments.multigrid.l0nu, 2);
        assert_eq!(arguments.multigrid.l1nu, Some(5));
    }
    let base = [
        "lmc",
        "current",
        "--resistance",
        "resistance.tif",
        "--source",
        "source.tif",
        "--ground",
        "ground.tif",
    ];
    for extra in [
        vec!["--precision", "mixed", "--explicit"],
        vec!["--tol", "NaN"],
        vec!["--max-iter", "0"],
        vec!["--l0nu", "0"],
        vec!["--coarseomega", "inf"],
        vec!["--precision", "f32", "--tol", "1e-100"],
        vec!["--precision", "invalid"],
    ] {
        let mut values = base.to_vec();
        values.extend(extra);
        assert!(parse(&values).is_err());
    }
}

#[test]
fn invalid_sampling_options_are_rejected() {
    let model = TestModel::new("base");
    let base = [
        "lmc",
        "resistance",
        "--model",
        &model.path,
        "--base",
        "data.tif",
        "--output-dir",
        "results",
    ];
    for extra in [
        vec!["--samples", "0"],
        vec!["--samples", "not-a-number"],
        vec!["--seed", "negative"],
        vec!["--save-samples"],
    ] {
        let mut values = base.to_vec();
        values.extend(extra);
        assert!(parse(&values).is_err());
    }
}
