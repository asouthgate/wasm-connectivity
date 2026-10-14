//! Native memory-profiling harness for the connectivity solve.
//!
//! Rasterizes GeoJSON, solves Jacobi / MG x Shunt / Dirichlet, and writes
//! ASC + PNG artifacts under a heap profiler (e.g. valgrind/massif).
//!
//! Usage:
//!   cargo run --profile release-prof --bin prof-solve --features bin -- <500|1000> \
//!       [--solver jacobi|mg|all] [--ground shunt|dirichlet|all] \
//!       [--mode artifacts|pipeline] [--out <dir>]
//!
//! `artifacts` (default) writes ASC+PNG per solver; `pipeline` runs the
//! browser-equivalent path (run_geospatial_pipeline_mg + serde_json
//! serialization) so profiling also accounts for serialization cost.

use std::fs;
use std::time::Instant;

use wasm_connect::raster::io::{asc_to_png, parse_asc, write_asc};
use wasm_connect::solve::{self, GroundMode};

const DATA_DIR: &str = "example/public/geodata";

fn run(resolution: usize, solver: &str, ground: &str, mode: &str, out_dir: &str) {
    fs::create_dir_all(out_dir).unwrap();

    let base = parse_asc(&format!("{DATA_DIR}/base_resistance_{resolution}.asc"));
    let src = parse_asc(&format!("{DATA_DIR}/source_{resolution}.asc"));
    let gnd = parse_asc(&format!("{DATA_DIR}/ground_{resolution}.asc"));
    let geojson = fs::read_to_string(format!("{DATA_DIR}/all_features_{resolution}.geojson")).unwrap();

    let layer_params_str =
        r#"{"roads":{"resistance":5,"width":3},"rivers":{"resistance":0.5,"width":4},"buildings":{"resistance":500,"width":0}}"#;

    // Browser-equivalent path: run the full pipeline and serialize the result
    // to JSON exactly like lib.rs::run_geospatial_pipeline_mg does, so
    // profiling also accounts for the serde_json serialization cost.
    if mode == "pipeline" {
        let gm = match ground {
            "shunt" => GroundMode::Shunt,
            _ => GroundMode::Dirichlet,
        };
        let t = Instant::now();
        let output = wasm_connect::geospatial::run_geospatial_pipeline_mg(
            &base.data, base.nrows, base.ncols, base.nodata,
            &geojson, layer_params_str,
            base.xllcorner, base.ymax, base.cellsize,
            &src.data, &gnd.data, 100_000, 1e-6, gm,
        );
        let json = serde_json::to_string(&output).unwrap();
        let path = format!("{out_dir}/pipeline_output.json");
        fs::write(&path, &json).unwrap();
        eprintln!(
            "[pipeline {gm:?}] {} iters, json {} bytes in {} ms",
            output.total_iters,
            json.len(),
            t.elapsed().as_millis()
        );
        return;
    }

    let (resistance, _, warnings) = wasm_connect::geospatial::prepare_geospatial_layers(
        &base.data, base.nrows, base.ncols,
        &geojson, layer_params_str,
        base.xllcorner, base.ymax, base.cellsize,
    );
    for w in &warnings {
        eprintln!("[rasterize warn] {w}");
    }

    let out_nodata = -9999.0;

    write_asc(
        &format!("{out_dir}/resistance_with_building.asc"),
        &resistance, base.nrows, base.ncols,
        base.xllcorner, base.yllcorner, base.cellsize, out_nodata,
    );

    let modes: Vec<(GroundMode, &str)> = match ground {
        "shunt" => vec![(GroundMode::Shunt, "shunt")],
        "dirichlet" => vec![(GroundMode::Dirichlet, "dirichlet")],
        _ => vec![(GroundMode::Shunt, "shunt"), (GroundMode::Dirichlet, "dirichlet")],
    };

    for &(ground_mode, suffix) in &modes {
        for name in ["jacobi", "mg"] {
            if solver != "all" && solver != name {
                continue;
            }
            let t = Instant::now();
            let result = match name {
                "jacobi" => solve::solve_raster_sources_jacobi(
                    &resistance, base.nrows, base.ncols, base.nodata,
                    &src.data, &gnd.data, 100_000, 1e-6, true, ground_mode,
                ),
                _ => solve::solve_raster_sources_mg(
                    &resistance, base.nrows, base.ncols, base.nodata,
                    &src.data, &gnd.data, 100_000, 1e-6, true, ground_mode,
                ),
            };
            assert!(result.total_iters < 100_000,
                "{name} CG reached the iteration limit ({suffix})");
            let path = format!("{out_dir}/current_map_{name}_{suffix}");
            write_asc(
                &format!("{path}.asc"),
                &result.output.current_map, base.nrows, base.ncols,
                base.xllcorner, base.yllcorner, base.cellsize, out_nodata,
            );
            asc_to_png(&format!("{path}.asc"), &format!("{path}.png"));
            eprintln!("[{name} {suffix}] {} iters in {} ms", result.total_iters, t.elapsed().as_millis());
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: prof_solve <500|1000> [--solver jacobi|mg|all] [--ground shunt|dirichlet|all] [--mode artifacts|pipeline] [--out <dir>]");
        std::process::exit(1);
    }

    let resolution: usize = args[1].parse().unwrap_or(500);
    let mut solver = "all".to_string();
    let mut ground = "all".to_string();
    let mut mode = "artifacts".to_string();
    let mut out_dir = "tests/output".to_string();

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
            "--mode" => {
                mode = args.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            "--out" => {
                out_dir = args.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            _ => i += 1,
        }
    }

    run(resolution, &solver, &ground, &mode, &out_dir);
}
