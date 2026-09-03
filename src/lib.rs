pub mod circuit;
pub mod geospatial;
pub mod linalg;
pub mod raster;
pub mod resistance;
pub mod roost;
pub mod solve;

pub use circuit::build_circuit_model;
pub use solve::cache;

use wasm_bindgen::prelude::*;
use serde::Serialize;
use serde_json::json;

pub const NODATA_SENTINEL: f64 = -9999.0;
pub const DEFAULT_MAX_ITER: usize = 100_000;
pub const DEFAULT_TOL: f64 = 1e-6;

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
pub fn solve_raster_sources_jacobi_cached(
    resistance_data: Vec<f64>,
    nrows: usize,
    ncols: usize,
    nodata: f64,
    source_data: Vec<f64>,
    ground_data: Vec<f64>,
    max_iter: usize,
    tol: f64,
    rebuild_laplacian: bool,
    use_dirichlet_ground: bool,
) -> String {
    let ground_mode = if use_dirichlet_ground { solve::GroundMode::Dirichlet } else { solve::GroundMode::Neumann };
    let annotated = solve::solve_raster_cached(
        &resistance_data,
        nrows,
        ncols,
        nodata,
        &source_data,
        &ground_data,
        max_iter,
        tol,
        true,
        rebuild_laplacian,
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
) -> String {
    let ground_mode = if use_dirichlet_ground { solve::GroundMode::Dirichlet } else { solve::GroundMode::Neumann };
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
pub fn reset_cache() {
    cache::reset();
}

#[wasm_bindgen]
pub fn run_geospatial_pipeline_cached_mg(
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
) -> String {
    let ground_mode = if use_dirichlet_ground { solve::GroundMode::Dirichlet } else { solve::GroundMode::Neumann };
    let output = geospatial::run_geospatial_pipeline_cached_mg(
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

#[derive(Serialize)]
struct RasterizeOutput {
    resistance_map: Vec<f64>,
    layer_masks: Vec<geospatial::LayerMask>,
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
) -> String {
    let params: resistance::pipeline::ResistanceParams = match serde_json::from_str(&params_json) {
        Ok(p) => p,
        Err(e) => {
            return serde_json::to_string(&json!({ "error": format!("Invalid params JSON: {}", e) }))
                .unwrap_or_else(|_| r#"{"error":"Invalid params JSON"}"#.to_string());
        }
    };
    // Zero-filled LCM used only for the NaN-masking pass in run_resistance_pipeline;
    // landscape conductance is supplied via the override so LCM conductance is not needed.
    let lcm = vec![0.0f64; params.nrows * params.ncols];
    let output = resistance::pipeline::run_resistance_pipeline(
        &road_binary, &river_binary, &building_mask, &lcm, &dtm, &dsm,

        &generic_resistance, &lamps, &params, Some(&landscape_conductance),
    );

    let mut map = serde_json::Map::new();
    map.insert("total_res".to_string(),    serde_json::Value::String(f64_to_base64(&output.total_res)));
    map.insert("lamp_res".to_string(),     serde_json::Value::String(f64_to_base64(&output.lamp_res)));
    map.insert("road_res".to_string(),     serde_json::Value::String(f64_to_base64(&output.road_res)));
    map.insert("river_res".to_string(),    serde_json::Value::String(f64_to_base64(&output.river_res)));
    map.insert("landscape_res".to_string(),serde_json::Value::String(f64_to_base64(&output.landscape_res)));
    map.insert("linear_res".to_string(),   serde_json::Value::String(f64_to_base64(&output.linear_res)));
    map.insert("generic_res".to_string(),  serde_json::Value::String(f64_to_base64(&output.generic_res)));
    map.insert("soft_surf".to_string(),    serde_json::Value::String(f64_to_base64(&output.soft_surf)));
    map.insert("hard_surf".to_string(),    serde_json::Value::String(f64_to_base64(&output.hard_surf)));
    map.insert("nrows".to_string(),        serde_json::json!(output.nrows));
    map.insert("ncols".to_string(),        serde_json::json!(output.ncols));
    serde_json::to_string(&map).unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
}

#[derive(Serialize)]
struct RoostSurfaceOutput {
    x: f64,
    y: f64,
    loss: f64,
    grid_size: usize,
    surface: Vec<f64>,
}

/// Estimate a bat roost location from per-detector call data using the
/// Henley et al. error-surface method. `surface` is the full `grid_size x grid_size`
/// loss field (row-major, y-outer), and `x`/`y`/`loss` are the best point.
#[wasm_bindgen]
pub fn compute_roost_surface(
    x: Vec<f64>,
    y: Vec<f64>,
    counts: Vec<f64>,
    grid_size: usize,
    capture_radius: f64,
    diffusivity: f64,
    t0: f64,
    t1: f64,
    loss: String,
) -> String {
    if loss != "l2" && loss != "l1" {
        return serde_json::to_string(&json!({ "error": format!("invalid loss {:?}, expected l2 or l1", loss) }))
            .unwrap_or_else(|_| r#"{"error":"invalid loss"}"#.to_string());
    }
    if grid_size < 2 {
        return serde_json::to_string(&json!({ "error": "grid_size must be >= 2" }))
            .unwrap_or_else(|_| r#"{"error":"invalid grid_size"}"#.to_string());
    }
    if !(t1 > t0 && t0 > 0.0) {
        return serde_json::to_string(&json!({ "error": "require 0 < t0 < t1" }))
            .unwrap_or_else(|_| r#"{"error":"invalid t0/t1"}"#.to_string());
    }
    if x.len() != y.len() || x.len() != counts.len() || x.is_empty() {
        return serde_json::to_string(&json!({ "error": "x, y and counts must be non-empty and equal length" }))
            .unwrap_or_else(|_| r#"{"error":"invalid detector data"}"#.to_string());
    }

    let mut surface = Vec::with_capacity(grid_size * grid_size);
    let result = roost::compute_error_surface(
        &x, &y, &counts, grid_size, capture_radius, diffusivity, t0, t1, &loss,
        |_cx, _cy, l| surface.push(l),
    );

    json_response(&RoostSurfaceOutput {
        x: result.x,
        y: result.y,
        loss: result.loss,
        grid_size,
        surface,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raster_edge_to_edge() {
        cache::reset();
        let size = 10;
        let n = size * size;
        let res_data = vec![1.0f64; n];
        let mut source_data = vec![0.0f64; n];
        let mut ground_data = vec![0.0f64; n];
        for row in 0..size {
            source_data[row * size] = 1.0;
            ground_data[row * size + (size - 1)] = 1.0;
        }
        let output = solve::solve_raster_cached(
            &res_data, size, size, NODATA_SENTINEL, &source_data, &ground_data,
            DEFAULT_MAX_ITER, DEFAULT_TOL, true, false, solve::GroundMode::Neumann,
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
        cache::reset();
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
        let output = solve::solve_raster_cached(
            &res_data, size, size, NODATA_SENTINEL, &source_data, &ground_data,
            DEFAULT_MAX_ITER, DEFAULT_TOL, true, false, solve::GroundMode::Neumann,
        ).output;
        let center_voltage = output.voltages[5 * size + 5];
        assert!(
            center_voltage > 0.0,
            "Center voltage should be positive for current source"
        );
    }
}
