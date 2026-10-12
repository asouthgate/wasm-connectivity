//! Profile one connectivity solve and emit one JSON object on stdout.
//!
//! Usage:
//!   cargo run --profile release-prof --features bin,instrumentation-profile \
//!       --bin instrumentation-profile -- 500 --solver mg-lowmem --ground neumann
//! Diagnostics go to stderr.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Serialize;

use wasm_connect::linalg::multigrid::MgOptions;
use wasm_connect::memory::{take_profile, InstrumentationProfile};
use wasm_connect::solve::{self, GroundMode};

const DATA_DIR: &str = "example/public/geodata";

struct AscGrid {
    data: Vec<f64>,
    nrows: usize,
    ncols: usize,
    xllcorner: f64,
    cellsize: f64,
    nodata: f64,
    ymax: f64,
}

fn parse_asc<P: AsRef<Path>>(path: P) -> AscGrid {
    let text = fs::read_to_string(path.as_ref())
        .unwrap_or_else(|e| panic!("cannot read {}: {}", path.as_ref().display(), e));

    let mut ncols = 0usize;
    let mut nrows = 0usize;
    let mut xllcorner = 0.0f64;
    let mut yllcorner = 0.0f64;
    let mut cellsize = 0.0f64;
    let mut nodata = -9999.0f64;
    let mut data = Vec::new();

    for (i, line) in BufReader::new(text.as_bytes()).lines().enumerate() {
        let line = line.unwrap_or_default();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if i < 6 {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 {
                let val = parts[1].parse::<f64>().unwrap_or(0.0);
                match parts[0].to_lowercase().as_str() {
                    "ncols" => ncols = val as usize,
                    "nrows" => nrows = val as usize,
                    "xllcorner" | "xllcenter" => xllcorner = val,
                    "yllcorner" | "yllcenter" => yllcorner = val,
                    "cellsize" => cellsize = val,
                    "nodata_value" => nodata = val,
                    _ => {}
                }
            }
        } else {
            for token in trimmed.split_whitespace() {
                data.push(token.parse::<f64>().unwrap_or(nodata));
            }
        }
    }

    assert!(nrows > 0 && ncols > 0, "ASC dimensions must be positive");
    assert_eq!(
        data.len(),
        nrows * ncols,
        "ASC cell count must match dimensions"
    );
    let ymax = yllcorner + nrows as f64 * cellsize;
    AscGrid {
        data,
        nrows,
        ncols,
        xllcorner,
        cellsize,
        nodata,
        ymax,
    }
}

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

const USAGE: &str = "Usage: instrumentation-profile RESOLUTION --solver jacobi|mg|mg-lowmem --ground neumann|dirichlet";

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
            "--ground" if matches!(value.as_str(), "neumann" | "dirichlet") => &mut ground,
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
        "neumann" => GroundMode::Neumann,
        "dirichlet" => GroundMode::Dirichlet,
        _ => unreachable!(),
    };
    wasm_connect::cache::reset();
    let result = match solver.as_str() {
        "jacobi" => solve::solve_raster_cached(
            &resistance,
            base.nrows,
            base.ncols,
            base.nodata,
            &src.data,
            &gnd.data,
            100_000,
            1e-6,
            true,
            false,
            ground_mode,
        ),
        "mg" => solve::solve_raster_sources_mg_with_options(
            &resistance,
            base.nrows,
            base.ncols,
            base.nodata,
            &src.data,
            &gnd.data,
            100_000,
            1e-6,
            true,
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
            100_000,
            1e-6,
            true,
            ground_mode,
            options,
        ),
        _ => unreachable!(),
    };
    emit(resolution, &solver, &ground, result.total_iters);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_requires_one_solver_and_ground() {
        for args in [
            vec![],
            vec!["bad"],
            vec!["0"],
            vec!["500"],
            vec!["500", "--solver"],
            vec!["500", "--solver", "mg"],
            vec!["500", "--ground", "neumann"],
            vec!["500", "--solver", "all", "--ground", "neumann"],
            vec!["500", "--solver", "mg-stencil", "--ground", "neumann"],
            vec!["500", "--solver", "mg", "--ground", "all"],
            vec![
                "500",
                "--solver",
                "mg",
                "--ground",
                "neumann",
                "--cholesky-size",
                "64",
            ],
            vec![
                "500", "--solver", "mg", "--solver", "jacobi", "--ground", "neumann",
            ],
        ] {
            let args: Vec<String> = args.into_iter().map(str::to_string).collect();
            assert!(parse_args(&args).is_err(), "{args:?}");
        }
        for solver in ["jacobi", "mg", "mg-lowmem"] {
            for ground in ["neumann", "dirichlet"] {
                let args = ["500", "--solver", solver, "--ground", ground].map(str::to_string);
                let parsed = parse_args(&args).unwrap();
                assert_eq!(parsed.resolution, 500);
                assert_eq!(parsed.solver, solver);
                assert_eq!(parsed.ground, ground);
            }
        }
    }
}
