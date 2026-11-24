//! Profile one connectivity solve on aligned input rasters and emit JSON on stdout.

use clap::{value_parser, Arg, ArgAction, Command};
use gdal::raster::GdalType;
use lm_connect::linalg::{
    multigrid::MgOptions,
    real::{scalar, Real},
};
use lm_connect::memory::{take_profile, InstrumentationProfile};
use lm_connect::raster::io::{read_raster_typed, require_aligned, Raster};
use lm_connect::solve::{self, AnnotatedOutput, GroundMode, RasterOutput};
use serde::Serialize;
use std::{io, path::PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Serialize)]
struct ProfileRecord {
    accounting_scope: &'static str,
    resolution: usize,
    source_resolution: usize,
    solver: String,
    ground: String,
    precision: String,
    total_iters: usize,
    profile: Option<InstrumentationProfile>,
}

fn raster_argument(name: &'static str, description: &'static str) -> Arg {
    Arg::new(name)
        .long(name)
        .required(true)
        .value_name("FILE")
        .value_parser(value_parser!(PathBuf))
        .help(description)
}

fn command() -> Command {
    Command::new("lmc-profile")
        .about("Profile one connectivity solve on aligned ASC or GeoTIFF rasters")
        .arg(raster_argument("resistance", "Resistance raster"))
        .arg(raster_argument("source", "Current source raster"))
        .arg(raster_argument("ground", "Current ground raster"))
        .arg(Arg::new("ground-mode").long("ground-mode").required(true)
            .value_parser(["shunt", "dirichlet"]))
        .arg(Arg::new("solver").long("solver").required(true).value_parser(["mg", "mg-lowmem"]))
        .arg(Arg::new("precision").long("precision").default_value("f64")
            .value_parser(["f32", "f64", "mixed"]))
        .arg(Arg::new("timings").long("timings").action(ArgAction::SetTrue))
        .arg(Arg::new("residuals").long("residuals").action(ArgAction::SetTrue))
        .after_help("Memory accounting goes to stdout. Timings and residuals go to stderr. Mixed precision requires --solver mg-lowmem.")
}

fn read_inputs<F: Real + GdalType>(
    arguments: &clap::ArgMatches,
) -> Result<(Raster<F>, Raster<F>, Raster<F>)> {
    let resistance_path = arguments.get_one::<PathBuf>("resistance").unwrap();
    let source_path = arguments.get_one::<PathBuf>("source").unwrap();
    let ground_path = arguments.get_one::<PathBuf>("ground").unwrap();
    let resistance = read_raster_typed::<F>(resistance_path)?;
    let source = read_raster_typed::<F>(source_path)?;
    let ground = read_raster_typed::<F>(ground_path)?;
    require_aligned(&resistance.grid, &source.grid, source_path)?;
    require_aligned(&resistance.grid, &ground.grid, ground_path)?;
    Ok((resistance, source, ground))
}

fn solve_standard<F: Real>(
    inputs: (Raster<F>, Raster<F>, Raster<F>),
    solver: &str,
    ground_mode: GroundMode,
    options: MgOptions,
) -> AnnotatedOutput<RasterOutput<F>> {
    let (resistance, mut source, ground) = inputs;
    let grid = resistance.grid;
    match solver {
        "mg" => solve::solve_raster_sources_mg_with_options(
            &resistance.data,
            grid.nrows,
            grid.ncols,
            scalar::<F>(-9999.0),
            &source.data,
            &ground.data,
            100_000,
            F::DEFAULT_TOLERANCE,
            true,
            ground_mode,
            options,
        ),
        "mg-lowmem" => solve::solve_raster_sources_mg_stencil_in_place_with_options(
            &resistance.data,
            grid.nrows,
            grid.ncols,
            scalar::<F>(-9999.0),
            &mut source.data,
            &ground.data,
            100_000,
            F::DEFAULT_TOLERANCE,
            true,
            ground_mode,
            options,
        ),
        _ => unreachable!(),
    }
}

fn solve_mixed(
    inputs: (Raster<f32>, Raster<f32>, Raster<f32>),
    ground_mode: GroundMode,
    options: MgOptions,
) -> AnnotatedOutput<RasterOutput<f64>> {
    let (resistance, mut source, ground) = inputs;
    let grid = resistance.grid;
    solve::mixed::solve_raster_sources_mg_mixed_with_options(
        &resistance.data,
        grid.nrows,
        grid.ncols,
        -9999.0,
        &mut source.data,
        &ground.data,
        100_000,
        <f32 as Real>::DEFAULT_TOLERANCE as f64,
        true,
        ground_mode,
        options,
    )
}

fn check_result<F: Real>(result: AnnotatedOutput<RasterOutput<F>>) -> Result<usize> {
    match result.error {
        Some(error) => Err(io::Error::other(error).into()),
        None => Ok(result.total_iters),
    }
}

fn run(arguments: &clap::ArgMatches) -> Result<()> {
    let solver = arguments.get_one::<String>("solver").unwrap();
    let ground = arguments.get_one::<String>("ground-mode").unwrap();
    let precision = arguments.get_one::<String>("precision").unwrap();
    let ground_mode = match ground.as_str() {
        "shunt" => GroundMode::Shunt,
        "dirichlet" => GroundMode::Dirichlet,
        _ => unreachable!(),
    };
    let options = MgOptions {
        verbose: arguments.get_flag("timings"),
        profile: arguments.get_flag("residuals"),
        ..MgOptions::default()
    };
    let (resolution, total_iters) = match precision.as_str() {
        "mixed" => {
            let inputs = read_inputs(arguments)?;
            (
                inputs.0.grid.ncols,
                check_result(solve_mixed(inputs, ground_mode, options))?,
            )
        }
        "f32" => {
            let inputs = read_inputs::<f32>(arguments)?;
            (
                inputs.0.grid.ncols,
                check_result(solve_standard(inputs, solver, ground_mode, options))?,
            )
        }
        _ => {
            let inputs = read_inputs::<f64>(arguments)?;
            (
                inputs.0.grid.ncols,
                check_result(solve_standard(inputs, solver, ground_mode, options))?,
            )
        }
    };
    let record = ProfileRecord {
        accounting_scope: "Native solver payloads; low-memory solver reuses the source input as RHS. Excludes rasterization, caller extras and WASM/JSON wrapper storage.",
        resolution,
        source_resolution: resolution,
        solver: solver.clone(),
        ground: ground.clone(),
        precision: precision.clone(),
        total_iters,
        profile: take_profile(),
    };
    println!("{}", serde_json::to_string_pretty(&record)?);
    Ok(())
}

fn main() {
    let arguments = command().get_matches();
    let solver = arguments.get_one::<String>("solver").unwrap();
    let precision = arguments.get_one::<String>("precision").unwrap();
    if precision == "mixed" && solver != "mg-lowmem" {
        command()
            .error(
                clap::error::ErrorKind::ArgumentConflict,
                "mixed precision requires --solver mg-lowmem",
            )
            .exit();
    }
    if let Err(error) = run(&arguments) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_paths_and_diagnostic_flags_are_explicit() {
        let arguments = [
            "lmc-profile",
            "--resistance",
            "resistance.asc",
            "--source",
            "source.asc",
            "--ground",
            "ground.asc",
            "--ground-mode",
            "shunt",
            "--solver",
            "mg-lowmem",
        ];
        let plain = command().try_get_matches_from(arguments).unwrap();
        assert!(!plain.get_flag("timings") && !plain.get_flag("residuals"));
        let traced = command()
            .try_get_matches_from(arguments.into_iter().chain([
                "--precision",
                "mixed",
                "--timings",
                "--residuals",
            ]))
            .unwrap();
        assert!(traced.get_flag("timings") && traced.get_flag("residuals"));
        assert_eq!(traced.get_one::<String>("precision").unwrap(), "mixed");
        assert!(command()
            .try_get_matches_from(&arguments[..arguments.len() - 2])
            .is_err());
    }
}
