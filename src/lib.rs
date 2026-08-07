pub mod grid;
pub mod graph;
pub mod laplacian;
pub mod components;
pub mod pcg;
pub mod current;
pub mod solve;
pub mod cache;
pub mod geospatial;
pub mod resample;
pub mod multigrid;
pub mod cholesky;
pub mod resistance;

use wasm_bindgen::prelude::*;
use serde::{Deserialize, Serialize};
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

pub fn build_circuit_model(resistance_data: &[f64], nrows: usize, ncols: usize, nodata: f64) -> (Vec<i32>, usize, graph::EdgeTriplets, sprs::CsMat<f64>, Vec<Vec<usize>>) {
    let conductance = grid::Grid::to_conductance(resistance_data, nrows, ncols, nodata);
    let (cell_to_node, num_nodes) = grid::build_cell_to_node(&conductance);
    let edges = graph::build_conductance_edges(&conductance, &cell_to_node);
    let laplacian = laplacian::build_laplacian(&edges, num_nodes);
    let components = components::find_connected_components(&laplacian, num_nodes);
    (cell_to_node, num_nodes, edges, laplacian, components)
}

#[derive(Serialize, Deserialize)]
pub struct ConnectivityOutput {
    pub resistance_matrix: Vec<Vec<f64>>,
    pub current_map: Vec<f64>,
    pub nrows: usize,
    pub ncols: usize,
    pub point_ids: Vec<i32>,
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
pub fn solve_raster_sources_mg_alcouffe(
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
    let annotated = solve::solve_raster_sources_mg_alcouffe(
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
    let output = resample::downsample_raster(&data, nrows, ncols, nodata, target_rows, target_cols);
    json_response(&output)
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
    let mut map = serde_json::Map::new();
    map.insert("resistance_map".to_string(), serde_json::Value::String(f64_to_base64(&resistance_data)));
    let masks: Vec<serde_json::Value> = layer_masks.iter().map(|m| {
        serde_json::json!({ "name": m.name, "data": f64_to_base64(&m.data) })
    }).collect();
    map.insert("layer_masks".to_string(), serde_json::Value::Array(masks));
    map.insert("nrows".to_string(), serde_json::json!(nrows));
    map.insert("ncols".to_string(), serde_json::json!(ncols));
    map.insert("warnings".to_string(), serde_json::json!(warnings));
    serde_json::to_string(&map).unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
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

#[allow(dead_code)]
fn compute_points(
    resistance_data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    point_data: Vec<i32>,
    max_iter: usize,
    tol: f64,
) -> ConnectivityOutput {
    let (cell_to_node, _num_nodes, _edges, laplacian, components) = build_circuit_model(resistance_data, nrows, ncols, nodata);

    let focal_points = grid::extract_focal_points(&point_data, nrows, ncols, &cell_to_node);

    let result = solve::compute_point_sources(
        &laplacian,
        &components,
        &focal_points,
        &cell_to_node,
        nrows,
        ncols,
        max_iter,
        tol,
    );

    ConnectivityOutput {
        resistance_matrix: result.resistance_matrix,
        current_map: result.current_map,
        nrows: result.nrows,
        ncols: result.ncols,
        point_ids: result.point_ids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_uniform_resistance_grid(size: usize, resistance: f64) -> (Vec<f64>, Vec<i32>) {
        let n = size * size;
        let res_data = vec![resistance; n];
        let mut point_data = vec![0i32; n];
        point_data[0] = 1;
        point_data[n - 1] = 2;
        (res_data, point_data)
    }

    fn make_corridor_grid() -> (Vec<f64>, Vec<i32>) {
        let size = 10;
        let n = size * size;
        let mut res_data = vec![10.0f64; n];
        for row in 0..size {
            for col in 0..size {
                if col == 5 {
                    res_data[row * size + col] = 1.0;
                }
            }
        }

        let mut point_data = vec![0i32; n];
        point_data[0] = 1;
        point_data[n - 1] = 2;

        (res_data, point_data)
    }

    #[test]
    fn test_uniform_grid_resistance() {
        let (res_data, point_data) = make_uniform_resistance_grid(5, 1.0);
        let output = compute_points(&res_data, 5, 5, NODATA_SENTINEL, point_data, DEFAULT_MAX_ITER, DEFAULT_TOL);

        assert_eq!(output.point_ids.len(), 2);
        assert_eq!(output.resistance_matrix.len(), 2);
        assert_eq!(output.resistance_matrix[0].len(), 2);

        let r = output.resistance_matrix[0][1];
        assert!(r > 0.0, "Resistance should be positive, got {}", r);
        assert_eq!(
            output.resistance_matrix[1][0], r,
            "Resistance matrix must be symmetric"
        );
        assert_eq!(output.resistance_matrix[0][0], 0.0);
        assert_eq!(output.resistance_matrix[1][1], 0.0);
    }

    #[test]
    fn test_uniform_current_map_symmetry() {
        let (res_data, point_data) = make_uniform_resistance_grid(5, 1.0);
        let output = compute_points(&res_data, 5, 5, NODATA_SENTINEL, point_data, DEFAULT_MAX_ITER, DEFAULT_TOL);

        assert_eq!(output.current_map.len(), 25);
        let has_current = output.current_map.iter().any(|&v| v > 0.0);
        assert!(has_current, "Current map should have non-zero values");
    }

    #[test]
    fn test_corridor_grid() {
        let (res_data, point_data) = make_corridor_grid();
        let output = compute_points(&res_data, 10, 10, NODATA_SENTINEL, point_data, DEFAULT_MAX_ITER, DEFAULT_TOL);

        assert!(output.resistance_matrix[0][1] > 0.0);

        let mid_row = 5;
        let col_low_res = output.current_map[mid_row * 10];
        let col_low_res2 = output.current_map[mid_row * 10 + 9];
        let col_high_res = output.current_map[mid_row * 10 + 5];

        assert!(
            col_high_res > col_low_res,
            "Current should be higher through low-resistance corridor: corridor={}, edge_left={}, edge_right={}",
            col_high_res,
            col_low_res,
            col_low_res2
        );
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
        let output = solve::compute_raster_sources(
            &res_data, size, size, NODATA_SENTINEL, &source_data, &ground_data,
            DEFAULT_MAX_ITER, DEFAULT_TOL, true, solve::GroundMode::Neumann,
        );
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
        let output = solve::compute_raster_sources(
            &res_data, size, size, NODATA_SENTINEL, &source_data, &ground_data,
            DEFAULT_MAX_ITER, DEFAULT_TOL, true, solve::GroundMode::Neumann,
        );
        let center_voltage = output.voltages[5 * size + 5];
        assert!(
            center_voltage > 0.0,
            "Center voltage should be positive for current source"
        );
    }
}
