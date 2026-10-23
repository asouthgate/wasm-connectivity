use crate::linalg::real::Real;
pub mod circuit;
pub mod geospatial;
pub mod linalg;
pub mod memory;
pub mod raster;
pub mod resistance;
pub mod roost;
pub mod solve;

pub use circuit::build_circuit_model;

use wasm_bindgen::prelude::*;
use serde::Serialize;
use serde_json::json;

pub const NODATA_SENTINEL: f64 = -9999.0;
pub const DEFAULT_MAX_ITER: usize = 100_000;
pub const DEFAULT_TOL: f64 = <f64 as Real>::DEFAULT_TOLERANCE;

fn json_response<T: Serialize>(output: &T) -> String {
    serde_json::to_string(output).unwrap_or_else(|e| {
        serde_json::to_string(&json!({ "error": e.to_string() }))
            .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
    })
}

const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_base64(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(BASE64_CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(BASE64_CHARS[((triple >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            BASE64_CHARS[((triple >> 6) & 0x3F) as usize] as char
        } else {
            b'=' as char
        });
        out.push(if chunk.len() > 2 {
            BASE64_CHARS[(triple & 0x3F) as usize] as char
        } else {
            b'=' as char
        });
    }
    out
}

fn f64_to_base64(data: &[f64]) -> String {
    let mut bytes = vec![0u8; data.len() * 4];
    for (i, &v) in data.iter().enumerate() {
        bytes[i * 4..(i + 1) * 4].copy_from_slice(&(v as f32).to_le_bytes());
    }
    encode_base64(&bytes)
}

#[wasm_bindgen(start)]
pub fn init_panic_hook() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub fn solve_raster_sources_jacobi(
    resistance_data: Vec<f64>,
    nrows: usize,
    ncols: usize,
    nodata: f64,
    source_data: Vec<f64>,
    ground_data: Vec<f64>,
    max_iter: usize,
    tol: f64,
    use_dirichlet_ground: bool,
) -> String {
    let ground_mode = if use_dirichlet_ground { solve::GroundMode::Dirichlet } else { solve::GroundMode::Shunt };
    let annotated = solve::solve_raster_sources_jacobi(
        &resistance_data,
        nrows,
        ncols,
        nodata,
        &source_data,
        &ground_data,
        max_iter,
        tol,
        true,
        ground_mode,
    );
    json_response(&annotated)
}

#[wasm_bindgen]
pub fn solve_raster_sources_mg(
    resistance_data: Vec<f64>,
    nrows: usize,
    ncols: usize,
    nodata: f64,
    source_data: Vec<f64>,
    ground_data: Vec<f64>,
    max_iter: usize,
    tol: f64,
    use_dirichlet_ground: bool,
) -> String { solve_raster_sources_mg_impl(resistance_data, nrows, ncols, nodata, source_data, ground_data, max_iter, tol, use_dirichlet_ground) }

#[wasm_bindgen]
pub fn solve_raster_sources_mg_f32(
    resistance_data: Vec<f32>,
    nrows: usize,
    ncols: usize,
    nodata: f32,
    source_data: Vec<f32>,
    ground_data: Vec<f32>,
    max_iter: usize,
    tol: f32,
    use_dirichlet_ground: bool,
) -> String { solve_raster_sources_mg_impl(resistance_data, nrows, ncols, nodata, source_data, ground_data, max_iter, tol, use_dirichlet_ground) }

fn solve_raster_sources_mg_impl<F: Real>(
    resistance_data: Vec<F>,
    nrows: usize,
    ncols: usize,
    nodata: F,
    source_data: Vec<F>,
    ground_data: Vec<F>,
    max_iter: usize,
    tol: F,
    use_dirichlet_ground: bool,
) -> String {
    let ground_mode = if use_dirichlet_ground { solve::GroundMode::Dirichlet } else { solve::GroundMode::Shunt };
    let annotated = solve::solve_raster_sources_mg(
        &resistance_data,
        nrows,
        ncols,
        nodata,
        &source_data,
        &ground_data,
        max_iter,
        tol,
        true,
        ground_mode,
    );
    json_response(&annotated)
}

#[wasm_bindgen]
pub fn solve_raster_sources_mg_stencil(
    resistance_data: Vec<f64>,
    nrows: usize,
    ncols: usize,
    nodata: f64,
    source_data: Vec<f64>,
    ground_data: Vec<f64>,
    max_iter: usize,
    tol: f64,
    use_dirichlet_ground: bool,
) -> String { solve_raster_sources_mg_stencil_impl(resistance_data, nrows, ncols, nodata, source_data, ground_data, max_iter, tol, use_dirichlet_ground) }

// Compute currents and check convergence using the f64 master voltage first.
// Only the browser output is rounded; reuse input buffers rather than allocate maps.
fn compact_mixed_maps(
    output: solve::RasterOutput<f64>, mut voltage: Vec<f32>, mut current: Vec<f32>,
) -> solve::RasterOutput<f32> {
    assert_eq!(voltage.len(), output.voltages.len());
    assert_eq!(current.len(), output.current_map.len());
    for (dest, &value) in voltage.iter_mut().zip(&output.voltages) { *dest = value as f32; }
    for (dest, &value) in current.iter_mut().zip(&output.current_map) { *dest = value as f32; }
    solve::RasterOutput {
        voltages: voltage, current_map: current, nrows: output.nrows, ncols: output.ncols,
    }
}

#[wasm_bindgen]
pub fn solve_raster_sources_mg_mixed(
    resistance_data: Vec<f32>, nrows: usize, ncols: usize, nodata: f32,
    mut source_data: Vec<f32>, ground_data: Vec<f32>, max_iter: usize, tol: f64,
    use_dirichlet_ground: bool,
) -> String {
    let ground_mode = if use_dirichlet_ground { solve::GroundMode::Dirichlet } else { solve::GroundMode::Shunt };
    let annotated = solve::mixed::solve_raster_sources_mg_mixed_with_options(
        &resistance_data, nrows, ncols, nodata, &mut source_data, &ground_data,
        max_iter, tol, true, ground_mode, linalg::multigrid::MgOptions::default(),
    );
    let annotated = solve::AnnotatedOutput {
        output: compact_mixed_maps(annotated.output, source_data, ground_data),
        total_iters: annotated.total_iters,
        error: annotated.error,
    };
    drop(resistance_data);
    json_response(&annotated)
}

#[wasm_bindgen]
pub fn run_geospatial_pipeline_mg_mixed(
    base_raster: Vec<f32>, nrows: usize, ncols: usize, nodata: f32,
    geojson_str: String, layer_params_str: String, xmin: f64, ymax: f64, cellsize: f64,
    mut source_data: Vec<f32>, ground_data: Vec<f32>, max_iter: usize, tol: f64,
    use_dirichlet_ground: bool,
) -> String {
    let ground_mode = if use_dirichlet_ground { solve::GroundMode::Dirichlet } else { solve::GroundMode::Shunt };
    let output = geospatial::run_geospatial_pipeline_mg_mixed(
        &base_raster, nrows, ncols, nodata, &geojson_str, &layer_params_str, xmin, ymax, cellsize,
        &mut source_data, &ground_data, max_iter, tol, ground_mode,
    );
    let maps = compact_mixed_maps(solve::RasterOutput {
        voltages: output.voltage_map, current_map: output.current_map, nrows, ncols,
    }, source_data, ground_data);
    let output = geospatial::GeospatialOutput {
        resistance_map: output.resistance_map, current_map: maps.current_map,
        voltage_map: maps.voltages, layer_masks: output.layer_masks,
        nrows, ncols, warnings: output.warnings, total_iters: output.total_iters, error: output.error,
    };
    drop(base_raster);
    json_response(&output)
}

#[wasm_bindgen]
pub fn solve_raster_sources_mg_stencil_f32(
    resistance_data: Vec<f32>,
    nrows: usize,
    ncols: usize,
    nodata: f32,
    source_data: Vec<f32>,
    ground_data: Vec<f32>,
    max_iter: usize,
    tol: f32,
    use_dirichlet_ground: bool,
) -> String { solve_raster_sources_mg_stencil_impl(resistance_data, nrows, ncols, nodata, source_data, ground_data, max_iter, tol, use_dirichlet_ground) }

fn solve_raster_sources_mg_stencil_impl<F: Real>(
    resistance_data: Vec<F>,
    nrows: usize,
    ncols: usize,
    nodata: F,
    mut source_data: Vec<F>,
    mut ground_data: Vec<F>,
    max_iter: usize,
    tol: F,
    use_dirichlet_ground: bool,
) -> String {
    let ground_mode = if use_dirichlet_ground { solve::GroundMode::Dirichlet } else { solve::GroundMode::Shunt };
    let mut annotated = solve::solve_raster_sources_mg_stencil_in_place_with_options(
        &resistance_data,
        nrows,
        ncols,
        nodata,
        &mut source_data,
        &ground_data,
        max_iter,
        tol,
        true,
        ground_mode,
        linalg::multigrid::MgOptions::default(),
    );
    // Reuse the input allocations for the returned maps, then release the
    // solver's output allocations before JSON needs a large contiguous buffer.
    source_data.copy_from_slice(&annotated.output.voltages);
    ground_data.copy_from_slice(&annotated.output.current_map);
    std::mem::swap(&mut source_data, &mut annotated.output.voltages);
    std::mem::swap(&mut ground_data, &mut annotated.output.current_map);
    drop(source_data);
    drop(ground_data);
    json_response(&annotated)
}

#[wasm_bindgen]
pub fn run_geospatial_pipeline_mg(
    base_raster: Vec<f64>,
    nrows: usize,
    ncols: usize,
    nodata: f64,
    geojson_str: String,
    layer_params_str: String,
    xmin: f64,
    ymax: f64,
    cellsize: f64,
    source_data: Vec<f64>,
    ground_data: Vec<f64>,
    max_iter: usize,
    tol: f64,
    use_dirichlet_ground: bool,
) -> String { run_geospatial_pipeline_mg_impl(base_raster, nrows, ncols, nodata, geojson_str, layer_params_str, xmin, ymax, cellsize, source_data, ground_data, max_iter, tol, use_dirichlet_ground) }

#[wasm_bindgen]
pub fn run_geospatial_pipeline_mg_f32(
    base_raster: Vec<f32>,
    nrows: usize,
    ncols: usize,
    nodata: f32,
    geojson_str: String,
    layer_params_str: String,
    xmin: f64,
    ymax: f64,
    cellsize: f64,
    source_data: Vec<f32>,
    ground_data: Vec<f32>,
    max_iter: usize,
    tol: f32,
    use_dirichlet_ground: bool,
) -> String { run_geospatial_pipeline_mg_impl(base_raster, nrows, ncols, nodata, geojson_str, layer_params_str, xmin, ymax, cellsize, source_data, ground_data, max_iter, tol, use_dirichlet_ground) }

fn run_geospatial_pipeline_mg_impl<F: Real>(
    base_raster: Vec<F>,
    nrows: usize,
    ncols: usize,
    nodata: F,
    geojson_str: String,
    layer_params_str: String,
    xmin: f64,
    ymax: f64,
    cellsize: f64,
    source_data: Vec<F>,
    ground_data: Vec<F>,
    max_iter: usize,
    tol: F,
    use_dirichlet_ground: bool,
) -> String {
    let ground_mode = if use_dirichlet_ground { solve::GroundMode::Dirichlet } else { solve::GroundMode::Shunt };
    let output = geospatial::run_geospatial_pipeline_mg(
        &base_raster,
        nrows,
        ncols,
        nodata,
        &geojson_str,
        &layer_params_str,
        xmin,
        ymax,
        cellsize,
        &source_data,
        &ground_data,
        max_iter,
        tol,
        ground_mode,
    );
    json_response(&output)
}

#[wasm_bindgen]
pub fn run_geospatial_pipeline_mg_stencil(
    base_raster: Vec<f64>,
    nrows: usize,
    ncols: usize,
    nodata: f64,
    geojson_str: String,
    layer_params_str: String,
    xmin: f64,
    ymax: f64,
    cellsize: f64,
    source_data: Vec<f64>,
    ground_data: Vec<f64>,
    max_iter: usize,
    tol: f64,
    use_dirichlet_ground: bool,
) -> String { run_geospatial_pipeline_mg_stencil_impl(base_raster, nrows, ncols, nodata, geojson_str, layer_params_str, xmin, ymax, cellsize, source_data, ground_data, max_iter, tol, use_dirichlet_ground) }

#[wasm_bindgen]
pub fn run_geospatial_pipeline_mg_stencil_f32(
    base_raster: Vec<f32>,
    nrows: usize,
    ncols: usize,
    nodata: f32,
    geojson_str: String,
    layer_params_str: String,
    xmin: f64,
    ymax: f64,
    cellsize: f64,
    source_data: Vec<f32>,
    ground_data: Vec<f32>,
    max_iter: usize,
    tol: f32,
    use_dirichlet_ground: bool,
) -> String { run_geospatial_pipeline_mg_stencil_impl(base_raster, nrows, ncols, nodata, geojson_str, layer_params_str, xmin, ymax, cellsize, source_data, ground_data, max_iter, tol, use_dirichlet_ground) }

fn run_geospatial_pipeline_mg_stencil_impl<F: Real>(
    base_raster: Vec<F>,
    nrows: usize,
    ncols: usize,
    nodata: F,
    geojson_str: String,
    layer_params_str: String,
    xmin: f64,
    ymax: f64,
    cellsize: f64,
    source_data: Vec<F>,
    ground_data: Vec<F>,
    max_iter: usize,
    tol: F,
    use_dirichlet_ground: bool,
) -> String {
    let ground_mode = if use_dirichlet_ground { solve::GroundMode::Dirichlet } else { solve::GroundMode::Shunt };
    let output = geospatial::run_geospatial_pipeline_mg_stencil(
        &base_raster,
        nrows,
        ncols,
        nodata,
        &geojson_str,
        &layer_params_str,
        xmin,
        ymax,
        cellsize,
        &source_data,
        &ground_data,
        max_iter,
        tol,
        ground_mode,
    );
    json_response(&output)
}

#[wasm_bindgen]
pub fn downsample_raster(
    data: Vec<f64>,
    nrows: usize,
    ncols: usize,
    nodata: f64,
    target_rows: usize,
    target_cols: usize,
) -> String {
    let output = raster::downsample_raster(&data, nrows, ncols, nodata, target_rows, target_cols);
    json_response(&output)
}

#[wasm_bindgen]
pub fn upsample_raster(
    data: Vec<f64>, nrows: usize, ncols: usize, target_rows: usize, target_cols: usize,
) -> Vec<f64> {
    raster::upsample_raster(&data, nrows, ncols, target_rows, target_cols)
}

#[derive(Serialize)]
struct RasterizeOutput<F: Real = f64> {
    resistance_map: Vec<F>,
    layer_masks: Vec<geospatial::LayerMask<F>>,
    nrows: usize,
    ncols: usize,
    warnings: Vec<String>,
}

#[wasm_bindgen]
pub fn rasterize_geojson(
    base_raster: Vec<f64>,
    nrows: usize,
    ncols: usize,
    _nodata: f64,
    geojson_str: String,
    layer_params_str: String,
    xmin: f64,
    ymax: f64,
    cellsize: f64,
) -> String { rasterize_geojson_impl(base_raster, nrows, ncols, _nodata, geojson_str, layer_params_str, xmin, ymax, cellsize) }

#[wasm_bindgen]
pub fn rasterize_geojson_f32(
    base_raster: Vec<f32>,
    nrows: usize,
    ncols: usize,
    _nodata: f32,
    geojson_str: String,
    layer_params_str: String,
    xmin: f64,
    ymax: f64,
    cellsize: f64,
) -> String { rasterize_geojson_impl(base_raster, nrows, ncols, _nodata, geojson_str, layer_params_str, xmin, ymax, cellsize) }

fn rasterize_geojson_impl<F: Real>(
    base_raster: Vec<F>,
    nrows: usize,
    ncols: usize,
    _nodata: F,
    geojson_str: String,
    layer_params_str: String,
    xmin: f64,
    ymax: f64,
    cellsize: f64,
) -> String {
    let (resistance_data, layer_masks, warnings) = geospatial::prepare_geospatial_layers(
        &base_raster, nrows, ncols, &geojson_str, &layer_params_str, xmin, ymax, cellsize,
    );
    let output = RasterizeOutput {
        resistance_map: resistance_data,
        layer_masks,
        nrows,
        ncols,
        warnings,
    };
    json_response(&output)
}

#[wasm_bindgen]
pub fn run_resistance_pipeline_browser(
    road_binary: Vec<f64>,
    river_binary: Vec<f64>,
    building_mask: Vec<f64>,
    dtm: Vec<f64>,
    dsm: Vec<f64>,
    generic_resistance: Vec<f64>,
    lamps: Vec<f64>,
    landscape_conductance: Vec<f64>,
    params_json: String,
) -> Result<BrowserResistanceOutput, String> {
    let params: resistance::pipeline::ResistanceParams = serde_json::from_str(&params_json)
        .map_err(|e| format!("Invalid params JSON: {}", e))?;
    let output = resistance::pipeline::run_resistance_pipeline_impl(
        &road_binary, &river_binary, &building_mask, None, &dtm, &dsm,
        &generic_resistance, &lamps, &params, Some(&landscape_conductance), None, false,
    );
    drop((road_binary, river_binary, building_mask, dtm, dsm,
        generic_resistance, lamps, landscape_conductance));
    Ok(resistance_output_to_binary(output))
}

/// Like [`run_resistance_pipeline_browser`], but skips the raycasted irradiance
/// and instead derives lamp resistance from a pre-computed light map (an
/// irradiance raster already aligned to the model grid, length `nrows*ncols`).
/// The light map is normalised and scaled by the lamp resistance factor inside
/// `run_resistance_pipeline`.
#[wasm_bindgen]
pub fn run_resistance_pipeline_browser_with_lightmap(
    road_binary: Vec<f64>,
    river_binary: Vec<f64>,
    building_mask: Vec<f64>,
    dtm: Vec<f64>,
    dsm: Vec<f64>,
    generic_resistance: Vec<f64>,
    lightmap: Vec<f64>,
    landscape_conductance: Vec<f64>,
    params_json: String,
) -> Result<BrowserResistanceOutput, String> {
    let params: resistance::pipeline::ResistanceParams = serde_json::from_str(&params_json)
        .map_err(|e| format!("Invalid params JSON: {}", e))?;
    let output = resistance::pipeline::run_resistance_pipeline_impl(
        &road_binary, &river_binary, &building_mask, None, &dtm, &dsm,
        &generic_resistance, &[], &params, Some(&landscape_conductance),
        Some(&lightmap), false,
    );
    drop((road_binary, river_binary, building_mask, dtm, dsm,
        generic_resistance, lightmap, landscape_conductance));
    Ok(resistance_output_to_binary(output))
}

#[wasm_bindgen]
pub struct BrowserResistanceOutput {
    rasters: [Vec<f64>; 9],
    nrows: usize,
    ncols: usize,
}

#[wasm_bindgen]
impl BrowserResistanceOutput {
    #[wasm_bindgen(getter)]
    pub fn nrows(&self) -> usize { self.nrows }

    #[wasm_bindgen(getter)]
    pub fn ncols(&self) -> usize { self.ncols }

    /// Consume one raster in browser field order, returning the existing f32 precision.
    pub fn take_raster(&mut self, index: usize) -> Result<Vec<f32>, String> {
        let raster = self.rasters.get_mut(index).ok_or("Unknown resistance raster")?;
        Ok(std::mem::take(raster).into_iter().map(|value| value as f32).collect())
    }
}

fn resistance_output_to_binary(output: resistance::pipeline::ResistanceOutput) -> BrowserResistanceOutput {
    BrowserResistanceOutput {
        rasters: [output.total_res, output.lamp_res, output.road_res, output.river_res,
            output.landscape_res, output.linear_res, output.generic_res, output.soft_surf, output.hard_surf],
        nrows: output.nrows,
        ncols: output.ncols,
    }
}

#[derive(Serialize)]
struct RoostDetectorPoint {
    x: f64,
    y: f64,
    count: f64,
}

#[derive(Serialize)]
struct RoostFinderOutput {
    /// Predicted roost (BNG easting/northing) + loss.
    x: f64,
    y: f64,
    loss: f64,
    /// Loss surface as base64-encoded little-endian `f32`, row-major with row 0
    /// at the north edge (matching the shared frontend plotter).
    surface_base64: String,
    /// Number of grid points per axis.
    grid_size: usize,
    /// Grid extents in BNG: `[xmin, ymin, xmax, ymax]`.
    bounds_bng: [f64; 4],
    /// Weighted-mean centroid of detectors by count (BNG).
    weighted_mean_x: f64,
    weighted_mean_y: f64,
    /// Per-detector points that contributed (BNG), with their (per-night or
    /// raw) counts.
    detectors: Vec<RoostDetectorPoint>,
    /// Non-fatal warnings (e.g. skipped detectors).
    warnings: Vec<String>,
}

/// Parse roost-finder CSVs, average calls per detector per active night, and
/// compute the error surface + predicted roost in one WASM call.
///
/// When `sunset_csv` is supplied (non-empty), master rows are filtered to
/// `[sunset, sunset + t1/60]`, so the observation window is tied to the
/// kernel's integration upper bound `t1`. `t0`/`t1` are the integration bounds
/// in seconds (defaults 0.01 / 5400).
///
/// Method: Henley et al. (2024), "A simple and fast method for estimating bat
/// roost locations", *Royal Society Open Science* 11(4): 231999.
/// <https://doi.org/10.1098/rsos.231999>
#[wasm_bindgen]
pub fn roost_finder_compute(
    detectors_csv: String,
    master_csv: String,
    sunset_csv: String,
    grid_size: usize,
    diffusivity: f64,
    t0: f64,
    t1: f64,
) -> String {
    run_roost_finder(
        &detectors_csv,
        &master_csv,
        &sunset_csv,
        grid_size,
        diffusivity,
        t0,
        t1,
    )
    .map(|out| json_response(&out))
    .unwrap_or_else(|e| {
        serde_json::to_string(&json!({ "error": e }))
            .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
    })
}

fn run_roost_finder(
    detectors_csv: &str,
    master_csv: &str,
    sunset_csv: &str,
    grid_size: usize,
    diffusivity: f64,
    t0: f64,
    t1: f64,
) -> Result<RoostFinderOutput, String> {
    use roost::io::{aggregate_with_warnings, count_calls, read_detectors, read_sunset};

    if grid_size < 2 {
        return Err("grid_size must be >= 2".to_string());
    }
    if !(t1 > t0 && t0 > 0.0) {
        return Err("require 0 < t0 < t1".to_string());
    }

    // The observation window is tied to t1: calls are counted from sunset to
    // sunset + t1/60 minutes, matching the kernel's integration upper bound.
    let minutes_after_sunset = t1 / 60.0;

    let detectors = read_detectors(detectors_csv)?;

    let sunset = if sunset_csv.trim().is_empty() {
        None
    } else {
        Some(read_sunset(sunset_csv)?)
    };

    let counts = count_calls(master_csv, sunset.as_ref().map(|s| (s, minutes_after_sunset)))?;
    let (agg, warnings) = aggregate_with_warnings(&detectors, &counts);

    if agg.x.is_empty() {
        return Err("no detectors with calls found after aggregation".to_string());
    }

    let mut surface = Vec::with_capacity(grid_size * grid_size);
    let result = roost::compute_error_surface(
        &agg.x, &agg.y, &agg.counts, grid_size, diffusivity, t0, t1,
        |_cx, _cy, l| surface.push(l),
    );

    let total: f64 = agg.counts.iter().sum();
    let weighted_mean_x = agg
        .x
        .iter()
        .zip(agg.counts.iter())
        .map(|(x, c)| x * c)
        .sum::<f64>()
        / total;
    let weighted_mean_y = agg
        .y
        .iter()
        .zip(agg.counts.iter())
        .map(|(y, c)| y * c)
        .sum::<f64>()
        / total;

    let detector_points = agg
        .x
        .iter()
        .zip(agg.y.iter())
        .zip(agg.counts.iter())
        .map(|((&x, &y), &count)| RoostDetectorPoint { x, y, count })
        .collect();

    // Grid extents match the search grid built by `compute_error_surface`.
    let xmin = agg.x.iter().cloned().fold(f64::INFINITY, f64::min);
    let xmax = agg.x.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let ymin = agg.y.iter().cloned().fold(f64::INFINITY, f64::min);
    let ymax = agg.y.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    // `surface` is south-first (iy = 0 -> ymin); the shared plotter expects
    // row 0 at the north edge, so flip the rows before encoding.
    let mut north_first = Vec::with_capacity(surface.len());
    for iy in (0..grid_size).rev() {
        north_first.extend_from_slice(&surface[iy * grid_size..(iy + 1) * grid_size]);
    }
    let surface_base64 = f64_to_base64(&north_first);

    Ok(RoostFinderOutput {
        x: result.x,
        y: result.y,
        loss: result.loss,
        surface_base64,
        grid_size,
        bounds_bng: [xmin, ymin, xmax, ymax],
        weighted_mean_x,
        weighted_mean_y,
        detectors: detector_points,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_browser_maps_reuse_f32_buffers_after_f64_current_calculation() {
        let mut source = vec![1.0_f32, 0.0, 0.0];
        let ground = vec![0.0_f32, 0.0, 1.0];
        let voltage_ptr = source.as_ptr();
        let current_ptr = ground.as_ptr();
        let result = solve::mixed::solve_raster_sources_mg_mixed_with_options(
            &[1e-6_f32; 3], 1, 3, -9999.0, &mut source, &ground, 1000, 1e-3,
            false, solve::GroundMode::Shunt, linalg::multigrid::MgOptions::default(),
        );
        assert!(result.error.is_none());
        let expected_voltage: Vec<f32> = result.output.voltages.iter().map(|&v| v as f32).collect();
        let expected_current: Vec<f32> = result.output.current_map.iter().map(|&v| v as f32).collect();
        let compact = compact_mixed_maps(result.output, source, ground);
        assert_eq!(compact.voltages.as_ptr(), voltage_ptr);
        assert_eq!(compact.current_map.as_ptr(), current_ptr);
        assert_eq!(compact.voltages, expected_voltage);
        assert_eq!(compact.current_map, expected_current);
        for &current in &compact.current_map { assert!((current - 1.0).abs() < 1e-3); }
        // Recomputing from rounded output voltages would lose the current accuracy.
        let rounded_current = solve::current::compute_current_map_from_raster(
            &[1e-6_f32; 3], &[0.0_f32, 0.0, 1.0], &compact.voltages, 1, 3, -9999.0, true,
        );
        assert!(rounded_current.iter().any(|&v| (v - 1.0).abs() > 0.01));
    }

    #[test]
    fn lowmem_wasm_buffer_reuse_preserves_serialized_output() {
        let (nr, nc, nodata) = (16, 16, 5.0);
        let resistance = vec![1.0; nr * nc];
        let mut source = vec![0.0; nr * nc];
        source[..6].copy_from_slice(&[1.0, f64::NAN, f64::INFINITY, -1.0, nodata, nodata + 5e-11]);
        let mut ground = vec![0.0; nr * nc];
        ground[8..12].copy_from_slice(&[f64::NAN, -2.0, nodata, nodata + 5e-11]);
        ground[nr * nc - 1] = 3.0;
        for dirichlet in [false, true] {
            let mode = if dirichlet { solve::GroundMode::Dirichlet } else { solve::GroundMode::Shunt };
            let expected = solve::solve_raster_sources_mg_stencil(
                &resistance, nr, nc, nodata, &source, &ground, 10000, 1e-8, true, mode,
            );
            let actual = solve_raster_sources_mg_stencil(
                resistance.clone(), nr, nc, nodata, source.clone(), ground.clone(),
                10000, 1e-8, dirichlet,
            );
            assert_eq!(actual, json_response(&expected));
        }
    }

    #[test]
    fn test_raster_edge_to_edge() {
        let size = 10;
        let n = size * size;
        let res_data = vec![1.0f64; n];
        let mut source_data = vec![0.0f64; n];
        let mut ground_data = vec![0.0f64; n];
        for row in 0..size {
            source_data[row * size] = 1.0;
            ground_data[row * size + (size - 1)] = 1.0;
        }
        let output = solve::solve_raster_sources_jacobi(
            &res_data, size, size, NODATA_SENTINEL, &source_data, &ground_data,
            DEFAULT_MAX_ITER, DEFAULT_TOL, true, solve::GroundMode::Shunt,
        ).output;
        assert_eq!(output.voltages.len(), n);
        assert_eq!(output.current_map.len(), n);
        let has_current = output.current_map.iter().any(|&v| v > 0.0);
        assert!(has_current, "Current map should have non-zero values");
        let has_voltage = output.voltages.iter().any(|&v| v.abs() > 0.0);
        assert!(has_voltage, "Voltage map should have non-zero values");
    }

    #[test]
    fn test_raster_center_source() {
        let size = 10;
        let n = size * size;
        let res_data = vec![1.0f64; n];
        let mut source_data = vec![0.0f64; n];
        let mut ground_data = vec![0.0f64; n];
        source_data[5 * size + 5] = 1.0;
        for row in 0..size {
            for col in 0..size {
                if row == 0 || row == size - 1 || col == 0 || col == size - 1 {
                    ground_data[row * size + col] = 1.0;
                }
            }
        }
        let output = solve::solve_raster_sources_jacobi(
            &res_data, size, size, NODATA_SENTINEL, &source_data, &ground_data,
            DEFAULT_MAX_ITER, DEFAULT_TOL, true, solve::GroundMode::Shunt,
        ).output;
        let center_voltage = output.voltages[5 * size + 5];
        assert!(
            center_voltage > 0.0,
            "Center voltage should be positive for current source"
        );
    }
}
