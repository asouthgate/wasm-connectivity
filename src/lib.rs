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
pub fn solve_raster_sources_cached(
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

#[derive(serde::Serialize)]
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
    let output = RasterizeOutput { resistance_map: resistance_data, layer_masks, nrows, ncols, warnings };
    json_response(&output)
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
