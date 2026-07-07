use std::collections::HashMap;
use std::fs;
use std::time::Instant;

mod common;

use wasm_connect::geospatial::{GeoTransform, LayerParams};
use wasm_connect::solve;

const DATA_DIR: &str = "web/public/geodata";
const OUT_DIR: &str = "tests/output";
const SIDE_PX: usize = 100;
const COL_START: usize = 200;
const ROW_START: usize = 200;

#[test]
#[ignore]
fn warm_start_with_building() {
    fs::create_dir_all(OUT_DIR).unwrap();

    let base = common::parse_asc(&format!("{DATA_DIR}/base_resistance_500.asc"));
    let src = common::parse_asc(&format!("{DATA_DIR}/source_500.asc"));
    let gnd = common::parse_asc(&format!("{DATA_DIR}/ground_500.asc"));
    let original_geojson =
        fs::read_to_string(format!("{DATA_DIR}/all_features_500.geojson")).unwrap();

    let layer_params_str =
        r#"{"roads":{"resistance":50,"width":3},"rivers":{"resistance":0.5,"width":4},"buildings":{"resistance":500,"width":0}}"#;

    let (res_without, _masks, warnings) = wasm_connect::geospatial::prepare_geospatial_layers(
        &base.data,
        base.nrows,
        base.ncols,
        &original_geojson,
        layer_params_str,
        base.xllcorner,
        base.ymax,
        base.cellsize,
    );
    for w in &warnings {
        eprintln!("[rasterize warn] {w}");
    }

    common::write_asc(
        &format!("{OUT_DIR}/resistance_without_building.asc"),
        &res_without,
        base.nrows,
        base.ncols,
        base.xllcorner,
        base.yllcorner,
        base.cellsize,
        base.nodata,
    );
    eprintln!("wrote resistance_without_building.asc");
    common::asc_to_png(
        &format!("{OUT_DIR}/resistance_without_building.asc"),
        &format!("{OUT_DIR}/resistance_without_building.png"),
    );

    let building_geojson =
        common::make_building_geojson(&base, COL_START, ROW_START, SIDE_PX);
    let mut building_params = HashMap::new();
    building_params.insert(
        "buildings".to_string(),
        LayerParams {
            resistance: 500.0,
            width: 0.0,
        },
    );
    let transform = GeoTransform {
        xmin: base.xllcorner,
        ymax: base.ymax,
        cellsize: base.cellsize,
    };
    let (res_with, _masks2) = wasm_connect::geospatial::rasterize_features(
        &building_geojson,
        &building_params,
        &res_without,
        base.nrows,
        base.ncols,
        &transform,
        &mut Vec::new(),
    );

    common::write_asc(
        &format!("{OUT_DIR}/resistance_with_building.asc"),
        &res_with,
        base.nrows,
        base.ncols,
        base.xllcorner,
        base.yllcorner,
        base.cellsize,
        base.nodata,
    );
    eprintln!("wrote resistance_with_building.asc");
    common::asc_to_png(
        &format!("{OUT_DIR}/resistance_with_building.asc"),
        &format!("{OUT_DIR}/resistance_with_building.png"),
    );

    // Phase 1 — cold solve on resistance without building
    wasm_connect::cache::reset();
    let t0 = Instant::now();
    let cold_without = solve::solve_raster_cached(
        &res_without,
        base.nrows,
        base.ncols,
        base.nodata,
        &src.data,
        &gnd.data,
        100_000,
        1e-6,
        true,
        false,
    );
    let t_cold_without = t0.elapsed();

    // Phase 2 — warm start on resistance with building (seeded from cache)
    let t1 = Instant::now();
    let warm_with = solve::solve_raster_cached(
        &res_with,
        base.nrows,
        base.ncols,
        base.nodata,
        &src.data,
        &gnd.data,
        100_000,
        1e-6,
        true,
        true, // rebuild=true → seeds PCG from prior voltage
    );
    let t_warm_with = t1.elapsed();

    // Phase 3 — cold solve on resistance with building (fresh, no cache)
    wasm_connect::cache::reset();
    let t2 = Instant::now();
    let cold_with = solve::solve_raster_cached(
        &res_with,
        base.nrows,
        base.ncols,
        base.nodata,
        &src.data,
        &gnd.data,
        100_000,
        1e-6,
        true,
        false,
    );
    let t_cold_with = t2.elapsed();

    // Verify warm vs cold reference produce the same current map
    let n = warm_with.output.current_map.len();
    let mut max_diff = 0.0f64;
    for i in 0..n {
        let diff = (warm_with.output.current_map[i] - cold_with.output.current_map[i]).abs();
        if diff > max_diff {
            max_diff = diff;
        }
    }
    assert!(
        max_diff < 1e-4,
        "warm vs cold current map diverge: max_diff={max_diff}"
    );

    // Save current maps
    let out_nodata = -9999.0;
    common::write_asc(
        &format!("{OUT_DIR}/current_map_cold_without.asc"),
        &cold_without.output.current_map,
        base.nrows,
        base.ncols,
        base.xllcorner,
        base.yllcorner,
        base.cellsize,
        out_nodata,
    );
    common::asc_to_png(
        &format!("{OUT_DIR}/current_map_cold_without.asc"),
        &format!("{OUT_DIR}/current_map_cold_without.png"),
    );
    common::write_asc(
        &format!("{OUT_DIR}/current_map_warm_with.asc"),
        &warm_with.output.current_map,
        base.nrows,
        base.ncols,
        base.xllcorner,
        base.yllcorner,
        base.cellsize,
        out_nodata,
    );
    common::asc_to_png(
        &format!("{OUT_DIR}/current_map_warm_with.asc"),
        &format!("{OUT_DIR}/current_map_warm_with.png"),
    );

    println!();
    println!("===== warm-start timing =====");
    println!(
        "cold (without building):  {:>8} ms   {:>6} iters",
        t_cold_without.as_millis(),
        cold_without.total_iters
    );
    println!(
        "warm (with building):     {:>8} ms   {:>6} iters",
        t_warm_with.as_millis(),
        warm_with.total_iters
    );
    println!(
        "cold ref (with building): {:>8} ms   {:>6} iters",
        t_cold_with.as_millis(),
        cold_with.total_iters
    );
    println!(
        "warm speed-up vs cold ref: {:.1}x",
        t_cold_with.as_secs_f64() / t_warm_with.as_secs_f64().max(0.001)
    );
    println!();
}

#[test]
#[ignore]
fn mg_vs_jacobi() {
    // Compare MG-preconditioned CG vs Jacobi-preconditioned CG on the
    // same filled resistance raster.  Both solvers use the identical
    // Laplacian (nodata filled → rectangular grid), so the results
    // should match.
    fs::create_dir_all(OUT_DIR).unwrap();

    let base = common::parse_asc(&format!("{DATA_DIR}/base_resistance_500.asc"));
    let src = common::parse_asc(&format!("{DATA_DIR}/source_500.asc"));
    let gnd = common::parse_asc(&format!("{DATA_DIR}/ground_500.asc"));
    let original_geojson =
        fs::read_to_string(format!("{DATA_DIR}/all_features_500.geojson")).unwrap();

    let layer_params_str =
        r#"{"roads":{"resistance":50,"width":3},"rivers":{"resistance":0.5,"width":4},"buildings":{"resistance":500,"width":0}}"#;

    let (res_without, _, _) = wasm_connect::geospatial::prepare_geospatial_layers(
        &base.data,
        base.nrows,
        base.ncols,
        &original_geojson,
        layer_params_str,
        base.xllcorner,
        base.ymax,
        base.cellsize,
    );

    // Add a building to the resistance
    let building_geojson =
        common::make_building_geojson(&base, COL_START, ROW_START, SIDE_PX);
    let mut building_params = HashMap::new();
    building_params.insert(
        "buildings".to_string(),
        LayerParams {
            resistance: 500.0,
            width: 0.0,
        },
    );
    let transform = GeoTransform {
        xmin: base.xllcorner,
        ymax: base.ymax,
        cellsize: base.cellsize,
    };
    let (res_with, _) = wasm_connect::geospatial::rasterize_features(
        &building_geojson,
        &building_params,
        &res_without,
        base.nrows,
        base.ncols,
        &transform,
        &mut Vec::new(),
    );

    // --- MG-preconditioned CG ---
    wasm_connect::cache::reset();
    let t0 = Instant::now();
    let mg_result = solve::solve_raster_sources_mg(
        &res_with,
        base.nrows,
        base.ncols,
        base.nodata,
        &src.data,
        &gnd.data,
        100_000,
        1e-6,
        true,
    );
    let t_mg = t0.elapsed();

    // --- Cold Jacobi CG (original pipeline, no nodata fill) ---
    wasm_connect::cache::reset();
    let t1 = Instant::now();
    let jacobi_result = solve::compute_raster_sources_annotated(
        &res_with,
        base.nrows,
        base.ncols,
        base.nodata,
        &src.data,
        &gnd.data,
        100_000,
        1e-6,
        true,
    );
    let t_jacobi = t1.elapsed();

    // --- Cold Jacobi CG on filled resistance (same Laplacian as MG) ---
    let filled = wasm_connect::multigrid::fill_nodata(&res_with, base.nodata);
    wasm_connect::cache::reset();
    let t2 = Instant::now();
    let jacobi_filled_result = solve::compute_raster_sources_annotated(
        &filled,
        base.nrows,
        base.ncols,
        base.nodata,
        &src.data,
        &gnd.data,
        100_000,
        1e-6,
        true,
    );
    let t_jacobi_filled = t2.elapsed();

    // Verify MG vs Jacobi-filled produce the same current map
    let n = mg_result.output.current_map.len();
    let mut max_diff = 0.0f64;
    for i in 0..n {
        let diff = (mg_result.output.current_map[i] - jacobi_filled_result.output.current_map[i]).abs();
        if diff > max_diff {
            max_diff = diff;
        }
    }
    assert!(
        max_diff < 1e-3,
        "MG vs Jacobi (filled) current map diverge: max_diff={max_diff}"
    );

    // Save MG current map
    let out_nodata = -9999.0;
    common::write_asc(
        &format!("{OUT_DIR}/current_map_mg.asc"),
        &mg_result.output.current_map,
        base.nrows,
        base.ncols,
        base.xllcorner,
        base.yllcorner,
        base.cellsize,
        out_nodata,
    );
    common::asc_to_png(
        &format!("{OUT_DIR}/current_map_mg.asc"),
        &format!("{OUT_DIR}/current_map_mg.png"),
    );

    println!();
    println!("===== MG vs Jacobi timing =====");
    println!(
        "MG-precond CG:          {:>8} ms   {:>6} iters",
        t_mg.as_millis(),
        mg_result.total_iters
    );
    println!(
        "Jacobi CG (original):   {:>8} ms   {:>6} iters",
        t_jacobi.as_millis(),
        jacobi_result.total_iters
    );
    println!(
        "Jacobi CG (filled):     {:>8} ms   {:>6} iters",
        t_jacobi_filled.as_millis(),
        jacobi_filled_result.total_iters
    );
    println!(
        "MG speed-up vs Jacobi (filled): {:.1}x",
        t_jacobi_filled.as_secs_f64() / t_mg.as_secs_f64().max(0.001)
    );
    println!();
}

#[test]
#[ignore]
fn mg_alcouffe_vs_bilinear() {
    fs::create_dir_all(OUT_DIR).unwrap();

    let base = common::parse_asc(&format!("{DATA_DIR}/base_resistance_500.asc"));
    let src = common::parse_asc(&format!("{DATA_DIR}/source_500.asc"));
    let gnd = common::parse_asc(&format!("{DATA_DIR}/ground_500.asc"));
    let original_geojson =
        fs::read_to_string(format!("{DATA_DIR}/all_features_500.geojson")).unwrap();

    let layer_params_str =
        r#"{"roads":{"resistance":50,"width":3},"rivers":{"resistance":0.5,"width":4},"buildings":{"resistance":500,"width":0}}"#;

    let (res_without, _, _) = wasm_connect::geospatial::prepare_geospatial_layers(
        &base.data,
        base.nrows,
        base.ncols,
        &original_geojson,
        layer_params_str,
        base.xllcorner,
        base.ymax,
        base.cellsize,
    );

    let building_geojson =
        common::make_building_geojson(&base, COL_START, ROW_START, SIDE_PX);
    let mut building_params = HashMap::new();
    building_params.insert(
        "buildings".to_string(),
        LayerParams {
            resistance: 500.0,
            width: 0.0,
        },
    );
    let transform = GeoTransform {
        xmin: base.xllcorner,
        ymax: base.ymax,
        cellsize: base.cellsize,
    };
    let (res_with, _) = wasm_connect::geospatial::rasterize_features(
        &building_geojson,
        &building_params,
        &res_without,
        base.nrows,
        base.ncols,
        &transform,
        &mut Vec::new(),
    );

    wasm_connect::cache::reset();
    let t0 = Instant::now();
    let bilinear_result = solve::solve_raster_sources_mg(
        &res_with,
        base.nrows,
        base.ncols,
        base.nodata,
        &src.data,
        &gnd.data,
        100_000,
        1e-6,
        true,
    );
    let t_bilinear = t0.elapsed();

    wasm_connect::cache::reset();
    let t1 = Instant::now();
    let alcouffe_result = solve::solve_raster_sources_mg_alcouffe(
        &res_with,
        base.nrows,
        base.ncols,
        base.nodata,
        &src.data,
        &gnd.data,
        100_000,
        1e-6,
        true,
    );
    let t_alcouffe = t1.elapsed();

    let n = bilinear_result.output.current_map.len();
    let mut max_diff = 0.0f64;
    for i in 0..n {
        let diff = (bilinear_result.output.current_map[i] - alcouffe_result.output.current_map[i]).abs();
        if diff > max_diff {
            max_diff = diff;
        }
    }
    assert!(
        max_diff < 1e-3,
        "Bilinear vs Alcouffe MG current map diverge: max_diff={max_diff}"
    );

    let out_nodata = -9999.0;
    common::write_asc(
        &format!("{OUT_DIR}/current_map_mg_bilinear.asc"),
        &bilinear_result.output.current_map,
        base.nrows,
        base.ncols,
        base.xllcorner,
        base.yllcorner,
        base.cellsize,
        out_nodata,
    );
    common::asc_to_png(
        &format!("{OUT_DIR}/current_map_mg_bilinear.asc"),
        &format!("{OUT_DIR}/current_map_mg_bilinear.png"),
    );
    common::write_asc(
        &format!("{OUT_DIR}/current_map_mg_alcouffe.asc"),
        &alcouffe_result.output.current_map,
        base.nrows,
        base.ncols,
        base.xllcorner,
        base.yllcorner,
        base.cellsize,
        out_nodata,
    );
    common::asc_to_png(
        &format!("{OUT_DIR}/current_map_mg_alcouffe.asc"),
        &format!("{OUT_DIR}/current_map_mg_alcouffe.png"),
    );

    println!();
    println!("===== MG Bilinear vs Alcouffe =====");
    println!(
        "Bilinear MG:  {:>8} ms  {:>6} iters",
        t_bilinear.as_millis(),
        bilinear_result.total_iters
    );
    println!(
        "Alcouffe MG:  {:>8} ms  {:>6} iters",
        t_alcouffe.as_millis(),
        alcouffe_result.total_iters
    );
    if alcouffe_result.total_iters < bilinear_result.total_iters {
        println!(
            "Alcouffe reduced iterations by {} ({:.0}%)",
            bilinear_result.total_iters - alcouffe_result.total_iters,
            (1.0 - alcouffe_result.total_iters as f64 / bilinear_result.total_iters as f64) * 100.0
        );
    }
    println!();
}
