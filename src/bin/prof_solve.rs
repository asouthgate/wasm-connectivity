//! Native memory-profiling harness for the connectivity solve.
//!
//! Runs the same pipeline as `tests/warm_start.rs::compute_all_current_maps`
//! (rasterize GeoJSON -> solve Jacobi / MG x Neumann / Dirichlet
//! -> write ASC + PNG artifacts) under a heap profiler (e.g. valgrind/massif).
//!
//! Usage:
//!   cargo run --profile release-prof --bin prof-solve --features bin -- <500|1000> \
//!       [--solver jacobi|mg|all] [--ground neumann|dirichlet|all] \
//!       [--mode artifacts|pipeline] [--out <dir>]
//!
//! `artifacts` (default) writes ASC+PNG per solver; `pipeline` runs the
//! browser-equivalent path (run_geospatial_pipeline_cached_mg + serde_json
//! serialization) so profiling also accounts for serialization cost.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::time::Instant;

use wasm_connect::solve::{self, GroundMode};

const DATA_DIR: &str = "example/public/geodata";

struct AscGrid {
    data: Vec<f64>,
    nrows: usize,
    ncols: usize,
    xllcorner: f64,
    yllcorner: f64,
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
    AscGrid { data, nrows, ncols, xllcorner, yllcorner, cellsize, nodata, ymax }
}

fn write_asc<P: AsRef<Path>>(
    path: P, data: &[f64], nrows: usize, ncols: usize,
    xllcorner: f64, yllcorner: f64, cellsize: f64, nodata: f64,
) {
    let mut f = fs::File::create(path.as_ref())
        .unwrap_or_else(|e| panic!("cannot create {}: {}", path.as_ref().display(), e));
    writeln!(f, "ncols {}", ncols).unwrap();
    writeln!(f, "nrows {}", nrows).unwrap();
    writeln!(f, "xllcorner {:.6}", xllcorner).unwrap();
    writeln!(f, "yllcorner {:.6}", yllcorner).unwrap();
    writeln!(f, "cellsize {:.6}", cellsize).unwrap();
    writeln!(f, "NODATA_value {}", nodata).unwrap();
    for row in 0..nrows {
        let mut line = String::new();
        for col in 0..ncols {
            if col > 0 {
                line.push(' ');
            }
            line.push_str(&format!("{:.6}", data[row * ncols + col]));
        }
        writeln!(f, "{}", line).unwrap();
    }
}

fn asc_to_png<P: AsRef<Path>, Q: AsRef<Path>>(asc_path: P, png_path: Q) {
    let grid = parse_asc(&asc_path);
    let n = grid.data.len();

    let mut min_val = f64::MAX;
    let mut max_val = f64::MIN;
    for &v in &grid.data {
        if v.is_finite() && v != grid.nodata {
            min_val = min_val.min(v);
            max_val = max_val.max(v);
        }
    }
    if max_val <= min_val {
        max_val = min_val + 1.0;
    }

    let scale = 255.0 / (max_val - min_val);
    let mut pixels = vec![0u8; n];
    for i in 0..n {
        let v = grid.data[i];
        if v.is_finite() && v != grid.nodata {
            let scaled = ((v - min_val) * scale).round() as u32;
            pixels[i] = scaled.min(255) as u8;
        }
    }

    let mut img = image::GrayImage::new(grid.ncols as u32, grid.nrows as u32);
    for row in 0..grid.nrows {
        for col in 0..grid.ncols {
            img.put_pixel(col as u32, row as u32, image::Luma([pixels[row * grid.ncols + col]]));
        }
    }
    img.save(&png_path).unwrap();
    eprintln!("wrote {}", png_path.as_ref().display());
}

fn run(resolution: usize, solver: &str, ground: &str, mode: &str, out_dir: &str) {
    fs::create_dir_all(out_dir).unwrap();

    let base = parse_asc(&format!("{DATA_DIR}/base_resistance_{resolution}.asc"));
    let src = parse_asc(&format!("{DATA_DIR}/source_{resolution}.asc"));
    let gnd = parse_asc(&format!("{DATA_DIR}/ground_{resolution}.asc"));
    let geojson = fs::read_to_string(format!("{DATA_DIR}/all_features_{resolution}.geojson")).unwrap();

    let layer_params_str =
        r#"{"roads":{"resistance":5,"width":3},"rivers":{"resistance":0.5,"width":4},"buildings":{"resistance":500,"width":0}}"#;

    // Browser-equivalent path: run the full pipeline and serialize the result
    // to JSON exactly like lib.rs::run_geospatial_pipeline_cached_mg does, so
    // profiling also accounts for the serde_json serialization cost.
    if mode == "pipeline" {
        let gm = match ground {
            "neumann" => GroundMode::Neumann,
            _ => GroundMode::Dirichlet,
        };
        let t = Instant::now();
        let output = wasm_connect::geospatial::run_geospatial_pipeline_cached_mg(
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
        "neumann" => vec![(GroundMode::Neumann, "neumann")],
        "dirichlet" => vec![(GroundMode::Dirichlet, "dirichlet")],
        _ => vec![(GroundMode::Neumann, "neumann"), (GroundMode::Dirichlet, "dirichlet")],
    };

    for &(ground_mode, suffix) in &modes {
        if solver == "jacobi" || solver == "all" {
            let t0 = Instant::now();
            wasm_connect::cache::reset();
            let jacobi = solve::solve_raster_cached(
                &resistance, base.nrows, base.ncols, base.nodata,
                &src.data, &gnd.data, 100_000, 1e-6, true, false, ground_mode,
            );
            write_asc(
                &format!("{out_dir}/current_map_jacobi_{suffix}.asc"),
                &jacobi.output.current_map, base.nrows, base.ncols,
                base.xllcorner, base.yllcorner, base.cellsize, out_nodata,
            );
            asc_to_png(
                &format!("{out_dir}/current_map_jacobi_{suffix}.asc"),
                &format!("{out_dir}/current_map_jacobi_{suffix}.png"),
            );
            eprintln!("[jacobi {suffix}] {} iters in {} ms", jacobi.total_iters, t0.elapsed().as_millis());
        }

        if solver == "mg" || solver == "all" {
            let t1 = Instant::now();
            let mg = solve::solve_raster_sources_mg(
                &resistance, base.nrows, base.ncols, base.nodata,
                &src.data, &gnd.data, 100_000, 1e-6, true, ground_mode,
            );
            write_asc(
                &format!("{out_dir}/current_map_mg_{suffix}.asc"),
                &mg.output.current_map, base.nrows, base.ncols,
                base.xllcorner, base.yllcorner, base.cellsize, out_nodata,
            );
            asc_to_png(
                &format!("{out_dir}/current_map_mg_{suffix}.asc"),
                &format!("{out_dir}/current_map_mg_{suffix}.png"),
            );
            eprintln!("[mg {suffix}] {} iters in {} ms", mg.total_iters, t1.elapsed().as_millis());
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: prof_solve <500|1000> [--solver jacobi|mg|all] [--ground neumann|dirichlet|all] [--mode artifacts|pipeline] [--out <dir>]");
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
