use super::{current::CurrentArguments, pipeline::PipelineArguments, SubcommandArguments};
use clap::{error::ErrorKind, value_parser, Arg, ArgAction, ArgMatches, Command};
use lm_connect::{
    linalg::{multigrid::MgOptions, real::Real},
    pipeline::{files::input_names, ExecutionOptions},
};
use std::{collections::HashMap, ffi::OsString, fs, path::PathBuf};

fn file_argument(name: &'static str, help: &'static str) -> Arg {
    Arg::new(name)
        .long(name)
        .required(true)
        .value_name("FILE")
        .value_parser(value_parser!(PathBuf))
        .help(help)
}

fn verbose_argument() -> Arg {
    Arg::new("verbose")
        .long("verbose")
        .short('v')
        .action(ArgAction::SetTrue)
        .help("Print execution details to stderr")
}

fn positive_integer(value: &str) -> Result<usize, String> {
    let number = value
        .parse::<usize>()
        .map_err(|_| "must be a positive integer")?;
    if number == 0 {
        return Err("must be a positive integer".into());
    }
    Ok(number)
}

fn positive_float(value: &str) -> Result<f64, String> {
    let number = value
        .parse::<f64>()
        .map_err(|_| "must be finite and positive")?;
    if !number.is_finite() || number <= 0.0 {
        return Err("must be finite and positive".into());
    }
    Ok(number)
}

fn build_pipeline_subcommand(
    name: &'static str,
    include_current: bool,
    names: &[String],
) -> Result<Command, clap::Error> {
    let description = if include_current {
        "Calculate resistance and current from pipeline JSON and input files"
    } else {
        "Calculate resistance from pipeline JSON and input files"
    };
    let mut command = Command::new(name)
        .about(description)
        .arg(file_argument("model", "Pipeline JSON definition"))
        .arg(
            Arg::new("output-dir")
                .long("output-dir")
                .required(true)
                .value_name("DIRECTORY")
                .value_parser(value_parser!(PathBuf))
                .help("Directory for output GeoTIFF maps"),
        )
        .arg(
            Arg::new("threads")
                .long("threads")
                .short('t')
                .value_parser(positive_integer)
                .default_value("1")
                .help("Number of worker threads for pipeline tasks"),
        )
        .arg(
            Arg::new("samples")
                .long("samples")
                .value_parser(positive_integer)
                .default_value("1")
                .help(
                    "Number of parameter draws; more than one returns means and sample variances",
                ),
        )
        .arg(
            Arg::new("seed")
                .long("seed")
                .value_parser(value_parser!(u64))
                .default_value("0")
                .help("Random seed for parameter draws"),
        )
        .arg(
            Arg::new("save-samples")
                .long("save-samples")
                .action(ArgAction::SetTrue)
                .help("Write all layer summaries and individual samples under samples/; requires at least two samples"),
        )
        .arg(
            Arg::new("all-layers")
                .long("all-layers")
                .action(ArgAction::SetTrue)
                .help("Also export named intermediate raster layers"),
        )
        .arg(verbose_argument())
        .after_help(concat!(
            "Input flags come from the model: --<input-name> FILE. ",
            "Pass --model FILE --help to list them.\n",
            "Raster grids must align and use a projected CRS in metres; ",
            "vector coordinates must use that CRS."
        ));
    if include_current {
        command = command
            .arg(file_argument(
                "source",
                "Current source raster, aligned with the pipeline grid",
            ))
            .arg(file_argument(
                "ground",
                "Current ground raster, aligned with the pipeline grid",
            ))
            .after_long_help(concat!(
                "Input flags come from the model: --<input-name> FILE. ",
                "Pass --model FILE --help to list them.\n",
                "Raster grids must align and use a projected CRS in metres; ",
                "vector coordinates must use that CRS.\n",
                "The current stage uses the f64 solver with shunt grounding."
            ));
    }
    for name in names {
        let conflicts = name == "help"
            || name == "version"
            || command
                .get_arguments()
                .any(|argument| argument.get_long() == Some(name.as_str()));
        if conflicts {
            return Err(command.error(
                ErrorKind::ArgumentConflict,
                format!("model input name {name} conflicts with a command-line option"),
            ));
        }
        command = command.arg(
            Arg::new(name.clone())
                .long(name.clone())
                .required(true)
                .value_name("FILE")
                .value_parser(value_parser!(PathBuf))
                .help("Model input file"),
        );
    }
    Ok(command)
}

fn build_current_subcommand() -> Command {
    let defaults = MgOptions::default();
    let mut command = Command::new("current")
        .about("Calculate current from an existing resistance raster")
        .arg(file_argument("resistance", "Resistance raster"))
        .arg(file_argument("source", "Current source raster"))
        .arg(file_argument("ground", "Current ground raster"))
        .arg(
            Arg::new("output")
                .long("output")
                .value_name("FILE")
                .value_parser(value_parser!(PathBuf))
                .default_value("rust_current.tif")
                .help("Output current map (.tif, .tiff, or .asc)"),
        )
        .arg(
            Arg::new("precision")
                .long("precision")
                .value_parser(["f32", "f64", "mixed"])
                .default_value("f64")
                .help("Solver precision; mixed uses f32 work arrays and f64 voltages"),
        )
        .arg(
            Arg::new("tol")
                .long("tol")
                .value_parser(positive_float)
                .help("Convergence tolerance [default: 1e-4 for f64, 1e-3 for f32/mixed]"),
        )
        .arg(
            Arg::new("max-iter")
                .long("max-iter")
                .value_parser(positive_integer)
                .default_value("100000")
                .help("Maximum CG iterations"),
        )
        .arg(
            Arg::new("explicit")
                .long("explicit")
                .action(ArgAction::SetTrue)
                .help("Store matrices explicitly; unavailable with mixed precision"),
        )
        .arg(verbose_argument())
        .after_help(concat!(
            "Inputs must be aligned GeoTIFF or ASC rasters. ",
            "GeoTIFF grids require a projected CRS in metres.\n",
            "ASC inputs without a CRS require an explicit .asc output. ",
            "Grounding uses shunt conductance."
        ));
    for (name, help, default) in [
        (
            "l0nu",
            "Finest-level pre/post smoothing sweeps",
            Some(defaults.l0nu),
        ),
        (
            "l1nu",
            "Second-level pre/post smoothing sweeps",
            defaults.l1nu,
        ),
        (
            "coarsenu",
            "Deeper nonterminal-level pre/post smoothing sweeps",
            Some(defaults.coarsenu),
        ),
    ] {
        let mut argument = Arg::new(name)
            .long(name)
            .value_parser(positive_integer)
            .help(help);
        if let Some(value) = default {
            argument = argument.default_value(value.to_string());
        }
        command = command.arg(argument);
    }
    for (name, help, default) in [
        ("l0omega", "Finest-level Jacobi damping", defaults.l0omega),
        (
            "coarseomega",
            "Coarser-level Jacobi damping",
            defaults.coarseomega,
        ),
    ] {
        let mut argument = Arg::new(name)
            .long(name)
            .value_parser(positive_float)
            .help(help);
        if let Some(value) = default {
            argument = argument.default_value(value.to_string());
        }
        command = command.arg(argument);
    }
    command.arg(
        Arg::new("l0postnu")
            .long("l0postnu")
            .value_parser(value_parser!(usize))
            .default_value(defaults.l0postnu.to_string())
            .help("Finest-level smoothing sweeps after CG"),
    )
}

fn build_cli(mode: Option<&str>, names: &[String]) -> Result<Command, clap::Error> {
    let resistance_inputs = if mode == Some("resistance") {
        names
    } else {
        &[]
    };
    let full_inputs = if mode == Some("full") { names } else { &[] };
    Ok(Command::new("lmc")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Calculate resistance and current maps")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(build_pipeline_subcommand(
            "resistance",
            false,
            resistance_inputs,
        )?)
        .subcommand(build_current_subcommand())
        .subcommand(build_pipeline_subcommand("full", true, full_inputs)?))
}

// Locate the model before parsing so clap can declare its input flags.
// Clap validates all arguments afterward.
fn find_model_path(arguments: &[OsString]) -> Option<PathBuf> {
    for (index, argument) in arguments.iter().enumerate().skip(2) {
        if argument == "--" {
            break;
        }
        if argument == "--model" {
            let value = arguments.get(index + 1)?;
            if value.to_str().is_some_and(|value| value.starts_with('-')) {
                return None;
            }
            return Some(PathBuf::from(value));
        }
        if let Some(value) = argument
            .to_str()
            .and_then(|value| value.strip_prefix("--model="))
        {
            return Some(PathBuf::from(value));
        }
    }
    None
}

fn required_path(matches: &ArgMatches, name: &str) -> PathBuf {
    matches
        .get_one::<PathBuf>(name)
        .unwrap_or_else(|| {
            panic!("missing --{name} after parsing; our CLI requires it or defines a default")
        })
        .clone()
}

fn parse_current_arguments(
    matches: &ArgMatches,
    command: &mut Command,
) -> Result<CurrentArguments, clap::Error> {
    let precision = matches
        .get_one::<String>("precision")
        .expect("missing --precision after parsing; our CLI defines a default")
        .clone();
    let explicit = matches.get_flag("explicit");
    if precision == "mixed" && explicit {
        return Err(command.error(
            ErrorKind::ArgumentConflict,
            "mixed precision uses the low-memory solver; remove --explicit",
        ));
    }
    let default_tolerance = if precision == "f64" {
        <f64 as Real>::DEFAULT_TOLERANCE
    } else {
        <f32 as Real>::DEFAULT_TOLERANCE as f64
    };
    let tolerance = matches
        .get_one::<f64>("tol")
        .copied()
        .unwrap_or(default_tolerance);
    if precision == "f32" && (!(tolerance as f32).is_finite() || tolerance as f32 <= 0.0) {
        return Err(command.error(
            ErrorKind::ValueValidation,
            "--tol must be representable and positive in f32",
        ));
    }
    Ok(CurrentArguments {
        resistance_path: required_path(matches, "resistance"),
        source_path: required_path(matches, "source"),
        ground_path: required_path(matches, "ground"),
        output_path: required_path(matches, "output"),
        precision,
        max_iter: *matches
            .get_one::<usize>("max-iter")
            .expect("missing --max-iter after parsing; our CLI defines a default"),
        tolerance,
        explicit,
        multigrid: MgOptions {
            l0nu: *matches
                .get_one::<usize>("l0nu")
                .expect("missing --l0nu after parsing; our CLI defines a default"),
            l1nu: matches.get_one::<usize>("l1nu").copied(),
            coarsenu: *matches
                .get_one::<usize>("coarsenu")
                .expect("missing --coarsenu after parsing; our CLI defines a default"),
            l0omega: matches.get_one::<f64>("l0omega").copied(),
            coarseomega: matches.get_one::<f64>("coarseomega").copied(),
            l0postnu: *matches
                .get_one::<usize>("l0postnu")
                .expect("missing --l0postnu after parsing; our CLI defines a default"),
            ..MgOptions::default()
        },
        verbose: matches.get_flag("verbose"),
    })
}

pub(super) fn parse(arguments: Vec<OsString>) -> Result<SubcommandArguments, clap::Error> {
    let mut command_without_inputs = build_cli(None, &[])?;
    let uses_pipeline = arguments
        .get(1)
        .is_some_and(|mode| mode == "resistance" || mode == "full");
    let mut pipeline_json = None;
    let mut names = Vec::new();
    // resistance and full modes require JSON specifications for pipeline specs
    if uses_pipeline {
        if let Some(path) = find_model_path(&arguments) {
            let json = fs::read_to_string(&path).map_err(|error| {
                command_without_inputs.error(ErrorKind::Io, format!("{}: {error}", path.display()))
            })?;
            names = input_names(&json).map_err(|error| {
                command_without_inputs.error(
                    ErrorKind::ValueValidation,
                    format!("{}: {error}", path.display()),
                )
            })?;
            pipeline_json = Some(json);
        }
    }
    let mode = arguments.get(1).and_then(|value| value.to_str());
    let mut cli = build_cli(mode, &names)?;
    let matches = cli.clone().try_get_matches_from(arguments)?;
    let (mode, matches) = matches
        .subcommand()
        .expect("a subcommand is required; clap should have rejected its absence");
    if mode == "current" {
        return Ok(SubcommandArguments::Current(parse_current_arguments(
            matches, &mut cli,
        )?));
    }
    let samples = *matches
        .get_one::<usize>("samples")
        .expect("missing --samples after parsing; our CLI defines a default");
    let save_samples = matches.get_flag("save-samples");
    if save_samples && samples < 2 {
        return Err(cli.error(
            ErrorKind::ValueValidation,
            "--save-samples requires at least two samples",
        ));
    }
    let mut input_paths = HashMap::new();
    for name in names {
        input_paths.insert(name.clone(), required_path(matches, &name));
    }
    let mut parsed = PipelineArguments {
        pipeline_json: pipeline_json
            .expect("pipeline JSON should have been loaded from the required --model argument"),
        input_paths,
        output_directory: required_path(matches, "output-dir"),
        execution_options: ExecutionOptions {
            threads: *matches
                .get_one::<usize>("threads")
                .expect("missing --threads after parsing; our CLI defines a default"),
            samples,
            seed: *matches
                .get_one::<u64>("seed")
                .expect("missing --seed after parsing; our CLI defines a default"),
            all_layers: matches.get_flag("all-layers") || save_samples,
            statistics: samples > 1,
        },
        save_samples,
        current_source_path: None,
        current_ground_path: None,
        verbose: matches.get_flag("verbose"),
    };
    if mode == "full" {
        parsed.current_source_path = Some(required_path(matches, "source"));
        parsed.current_ground_path = Some(required_path(matches, "ground"));
        Ok(SubcommandArguments::Full(parsed))
    } else {
        Ok(SubcommandArguments::Resistance(parsed))
    }
}
