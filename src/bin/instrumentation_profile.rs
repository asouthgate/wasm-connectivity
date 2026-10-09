//! Native instrumentation-profile harness for the connectivity solve.
//!
//! Runs the same pipeline as `prof_solve` (rasterize GeoJSON -> solve MG /
//! Jacobi x Neumann / Dirichlet) and, for each solve, emits the analytical
//! memory profile produced by the `instrumentation-profile` feature as NDJSON on stdout.
//!
//! Usage:
//!   cargo run --profile release-prof --bin instrumentation-profile \
//!       --features bin,instrumentation-profile -- [500|1000] \
//!       [--solver jacobi|mg|mg-stencil|all] [--ground neumann|dirichlet|all]
//!       [--cholesky-size N]
//!
//! Each output line is a JSON object:
//!   { "resolution", "solver", "ground", "total_iters", "profile": <InstrumentationProfile> }

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
    println!("{}", serde_json::to_string(&rec).unwrap());
}

const USAGE: &str = "Usage: instrumentation-profile RESOLUTION [--solver jacobi|mg|mg-stencil|all] [--ground neumann|dirichlet|all] [--cholesky-size N]";

struct Args {
    resolution: usize,
    solver: String,
    ground: String,
    cholesky_size: Option<usize>,
}

fn positive(value: &str, name: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|&n| n > 0)
        .ok_or_else(|| format!("{name} must be a positive integer"))
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let resolution = positive(args.first().ok_or("resolution is required")?, "resolution")?;
    let mut parsed = Args {
        resolution,
        solver: "all".into(),
        ground: "all".into(),
        cholesky_size: None,
    };
    let mut i = 1;
    while i < args.len() {
        let flag = args[i].as_str();
        if !matches!(flag, "--solver" | "--ground" | "--cholesky-size") {
            return Err(format!("unknown argument: {flag}"));
        }
        let value = args
            .get(i + 1)
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag {
            "--solver" if matches!(value.as_str(), "jacobi" | "mg" | "mg-stencil" | "all") => {
                parsed.solver = value.clone()
            }
            "--ground" if matches!(value.as_str(), "neumann" | "dirichlet" | "all") => {
                parsed.ground = value.clone()
            }
            "--cholesky-size" => parsed.cholesky_size = Some(positive(value, "Cholesky size")?),
            _ => return Err(format!("unsupported value for {flag}: {value}")),
        }
        i += 2;
    }
    if parsed.solver == "jacobi" && parsed.cholesky_size.is_some() {
        return Err("--cholesky-size requires an MG solver".into());
    }
    Ok(parsed)
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
        cholesky_size,
    } = parse_args(&arguments).unwrap_or_else(|error| {
        eprintln!("{error}\n{USAGE}");
        std::process::exit(2);
    });
    let options = MgOptions {
        cholesky_size,
        ..MgOptions::default()
    };

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

    let modes: Vec<(GroundMode, &str)> = match ground.as_str() {
        "neumann" => vec![(GroundMode::Neumann, "neumann")],
        "dirichlet" => vec![(GroundMode::Dirichlet, "dirichlet")],
        _ => vec![
            (GroundMode::Neumann, "neumann"),
            (GroundMode::Dirichlet, "dirichlet"),
        ],
    };

    for &(ground_mode, suffix) in &modes {
        if solver == "jacobi" || solver == "all" {
            wasm_connect::cache::reset();
            let jacobi = solve::solve_raster_cached(
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
            );
            emit(resolution, "jacobi", suffix, jacobi.total_iters);
        }

        if solver == "mg" || solver == "all" {
            wasm_connect::cache::reset();
            let mg = solve::solve_raster_sources_mg_with_options(
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
            );
            emit(resolution, "mg", suffix, mg.total_iters);
        }

        if solver == "mg-stencil" || solver == "all" {
            wasm_connect::cache::reset();
            let mg = solve::solve_raster_sources_mg_stencil_with_options(
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
            );
            emit(resolution, "mg-stencil", suffix, mg.total_iters);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_requires_valid_explicit_arguments() {
        for args in [
            vec![],
            vec!["bad"],
            vec!["0"],
            vec!["500", "--solver"],
            vec!["500", "--solver", "typo"],
            vec!["500", "--ground", "typo"],
            vec!["500", "--cholesky-size", "0"],
            vec!["500", "--unknown", "1"],
            vec!["500", "--solver", "jacobi", "--cholesky-size", "49"],
        ] {
            let args: Vec<String> = args.into_iter().map(str::to_string).collect();
            assert!(parse_args(&args).is_err());
        }
        let args = [
            "1000",
            "--solver",
            "mg-stencil",
            "--ground",
            "dirichlet",
            "--cholesky-size",
            "64",
        ]
        .map(str::to_string);
        let parsed = parse_args(&args).unwrap();
        assert_eq!(parsed.resolution, 1000);
        assert_eq!(parsed.solver, "mg-stencil");
        assert_eq!(parsed.ground, "dirichlet");
        assert_eq!(parsed.cholesky_size, Some(64));
        assert_eq!(parse_args(&["500".into()]).unwrap().cholesky_size, None);
    }
}
