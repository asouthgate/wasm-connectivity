//! Native memory-story harness for the connectivity solve.
//!
//! Runs the same pipeline as `prof_solve` (rasterize GeoJSON -> solve MG /
//! Jacobi x Neumann / Dirichlet) and, for each solve, emits the analytical
//! memory story produced by the `memory-story` feature as NDJSON on stdout.
//!
//! Usage:
//!   cargo run --profile release-prof --bin mem-story \
//!       --features bin,memory-story -- [500|1000] \
//!       [--solver jacobi|mg|mg-stencil|all] [--ground neumann|dirichlet|all]
//!
//! Each output line is a JSON object:
//!   { "resolution", "solver", "ground", "total_iters", "story": <MemoryStory> }

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Serialize;

use wasm_connect::memory::{take_story, MemoryStory};
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

    let ymax = yllcorner + nrows as f64 * cellsize;
    AscGrid { data, nrows, ncols, xllcorner, cellsize, nodata, ymax }
}

#[derive(Serialize)]
struct StoryRecord {
    resolution: usize,
    solver: String,
    ground: String,
    total_iters: usize,
    story: Option<MemoryStory>,
}

fn emit(resolution: usize, solver: &str, ground: &str, total_iters: usize) {
    let rec = StoryRecord {
        resolution,
        solver: solver.to_string(),
        ground: ground.to_string(),
        total_iters,
        story: take_story(),
    };
    println!("{}", serde_json::to_string(&rec).unwrap());
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let resolution: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(500);
    let mut solver = "all".to_string();
    let mut ground = "all".to_string();

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--solver" => {
                solver = args.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            "--ground" => {
                ground = args.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            _ => i += 1,
        }
    }

    let base = parse_asc(&format!("{DATA_DIR}/base_resistance_{resolution}.asc"));
    let src = parse_asc(&format!("{DATA_DIR}/source_{resolution}.asc"));
    let gnd = parse_asc(&format!("{DATA_DIR}/ground_{resolution}.asc"));
    let geojson = fs::read_to_string(format!("{DATA_DIR}/all_features_{resolution}.geojson")).unwrap();

    let layer_params_str =
        r#"{"roads":{"resistance":5,"width":3},"rivers":{"resistance":0.5,"width":4},"buildings":{"resistance":500,"width":0}}"#;

    let (resistance, _, warnings) = wasm_connect::geospatial::prepare_geospatial_layers(
        &base.data, base.nrows, base.ncols,
        &geojson, layer_params_str,
        base.xllcorner, base.ymax, base.cellsize,
    );
    for w in &warnings {
        eprintln!("[rasterize warn] {w}");
    }

    let modes: Vec<(GroundMode, &str)> = match ground.as_str() {
        "neumann" => vec![(GroundMode::Neumann, "neumann")],
        "dirichlet" => vec![(GroundMode::Dirichlet, "dirichlet")],
        _ => vec![(GroundMode::Neumann, "neumann"), (GroundMode::Dirichlet, "dirichlet")],
    };

    for &(ground_mode, suffix) in &modes {
        if solver == "jacobi" || solver == "all" {
            wasm_connect::cache::reset();
            let jacobi = solve::solve_raster_cached(
                &resistance, base.nrows, base.ncols, base.nodata,
                &src.data, &gnd.data, 100_000, 1e-6, true, false, ground_mode,
            );
            emit(resolution, "jacobi", suffix, jacobi.total_iters);
        }

        if solver == "mg" || solver == "all" {
            wasm_connect::cache::reset();
            let mg = solve::solve_raster_sources_mg(
                &resistance, base.nrows, base.ncols, base.nodata,
                &src.data, &gnd.data, 100_000, 1e-6, true, ground_mode,
            );
            emit(resolution, "mg", suffix, mg.total_iters);
        }

        if solver == "mg-stencil" || solver == "all" {
            wasm_connect::cache::reset();
            let mg = solve::solve_raster_sources_mg_stencil(
                &resistance, base.nrows, base.ncols, base.nodata,
                &src.data, &gnd.data, 100_000, 1e-6, true, ground_mode,
            );
            emit(resolution, "mg-stencil", suffix, mg.total_iters);
        }
    }
}
