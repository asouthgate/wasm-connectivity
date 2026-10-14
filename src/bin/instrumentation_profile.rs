//! Profile one connectivity solve and emit one JSON object on stdout.
//!
//! Usage:
//!   cargo run --profile release-prof --features bin,instrumentation-profile \
//!       --bin instrumentation-profile -- 500 --solver mg-lowmem --ground shunt
//! Diagnostics go to stderr.

use std::fs;

use serde::Serialize;

use wasm_connect::linalg::multigrid::MgOptions;
use wasm_connect::raster::io::parse_asc;
use wasm_connect::memory::{take_profile, InstrumentationProfile};
use wasm_connect::solve::{self, GroundMode};

const DATA_DIR: &str = "example/public/geodata";

#[derive(Serialize)]
struct ProfileRecord {
    resolution: usize,
    solver: String,
    ground: String,
    total_iters: usize,
    profile: Option<InstrumentationProfile>,
}

fn emit(resolution: usize, solver: &str, ground: &str, total_iters: usize) {
    let rec = ProfileRecord {
        resolution,
        solver: solver.to_string(),
        ground: ground.to_string(),
        total_iters,
        profile: take_profile(),
    };
    println!("{}", serde_json::to_string_pretty(&rec).unwrap());
}

const USAGE: &str = "Usage: instrumentation-profile RESOLUTION --solver jacobi|mg|mg-lowmem --ground shunt|dirichlet";

struct Args {
    resolution: usize,
    solver: String,
    ground: String,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let resolution = args
        .first()
        .ok_or("resolution is required")?
        .parse::<usize>()
        .ok()
        .filter(|&n| n > 0)
        .ok_or("resolution must be a positive integer")?;
    let mut solver = None;
    let mut ground = None;
    let mut i = 1;
    while i < args.len() {
        let flag = args[i].as_str();
        if !matches!(flag, "--solver" | "--ground") {
            return Err(format!("unknown argument: {flag}"));
        }
        let value = args
            .get(i + 1)
            .ok_or_else(|| format!("missing value for {flag}"))?;
        let target = match flag {
            "--solver" if matches!(value.as_str(), "jacobi" | "mg" | "mg-lowmem") => &mut solver,
            "--ground" if matches!(value.as_str(), "shunt" | "dirichlet") => &mut ground,
            _ => return Err(format!("unsupported value for {flag}: {value}")),
        };
        if target.replace(value.clone()).is_some() {
            return Err(format!("duplicate argument: {flag}"));
        }
        i += 2;
    }
    Ok(Args {
        resolution,
        solver: solver.ok_or("--solver is required")?,
        ground: ground.ok_or("--ground is required")?,
    })
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.len() == 1 && matches!(arguments[0].as_str(), "--help" | "-h") {
        println!("{USAGE}");
        return;
    }
    let Args {
        resolution,
        solver,
        ground,
    } = parse_args(&arguments).unwrap_or_else(|error| {
        eprintln!("{error}\n{USAGE}");
        std::process::exit(2);
    });
    let options = MgOptions::default();

    let base = parse_asc(&format!("{DATA_DIR}/base_resistance_{resolution}.asc"));
    let src = parse_asc(&format!("{DATA_DIR}/source_{resolution}.asc"));
    let gnd = parse_asc(&format!("{DATA_DIR}/ground_{resolution}.asc"));
    for grid in [&base, &src, &gnd] {
        assert!(grid.nrows > 0 && grid.ncols > 0, "ASC dimensions must be positive");
        assert_eq!(grid.data.len(), grid.nrows * grid.ncols,
            "ASC cell count must match dimensions");
    }
    assert_eq!(
        (src.nrows, src.ncols),
        (base.nrows, base.ncols),
        "source grid dimensions differ"
    );
    assert_eq!(
        (gnd.nrows, gnd.ncols),
        (base.nrows, base.ncols),
        "ground grid dimensions differ"
    );
    let geojson =
        fs::read_to_string(format!("{DATA_DIR}/all_features_{resolution}.geojson")).unwrap();

    let layer_params_str = r#"{"roads":{"resistance":5,"width":3},"rivers":{"resistance":0.5,"width":4},"buildings":{"resistance":500,"width":0}}"#;

    let (resistance, _, warnings) = wasm_connect::geospatial::prepare_geospatial_layers(
        &base.data,
        base.nrows,
        base.ncols,
        &geojson,
        layer_params_str,
        base.xllcorner,
        base.ymax,
        base.cellsize,
    );
    for w in &warnings {
        eprintln!("[rasterize warn] {w}");
    }

    let ground_mode = match ground.as_str() {
        "shunt" => GroundMode::Shunt,
        "dirichlet" => GroundMode::Dirichlet,
        _ => unreachable!(),
    };
    let max_iter = 100_000;
    let tol = 1e-6;
    let remove_average = true;
    let result = match solver.as_str() {
        "jacobi" => solve::solve_raster_sources_jacobi(
            &resistance,
            base.nrows,
            base.ncols,
            base.nodata,
            &src.data,
            &gnd.data,
            max_iter,
            tol,
            remove_average,
            ground_mode,
        ),
        "mg" => solve::solve_raster_sources_mg_with_options(
            &resistance,
            base.nrows,
            base.ncols,
            base.nodata,
            &src.data,
            &gnd.data,
            max_iter,
            tol,
            remove_average,
            ground_mode,
            options,
        ),
        "mg-lowmem" => solve::solve_raster_sources_mg_stencil_with_options(
            &resistance,
            base.nrows,
            base.ncols,
            base.nodata,
            &src.data,
            &gnd.data,
            max_iter,
            tol,
            remove_average,
            ground_mode,
            options,
        ),
        _ => unreachable!(),
    };
    emit(resolution, &solver, &ground, result.total_iters);
}
