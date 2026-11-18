use super::output::json_response;
use crate::{geospatial, raster};
use wasm_bindgen::prelude::*;

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

/// Rasterize GeoJSON `properties.value` fields. Returns row-major values;
/// uncovered cells use zero and overlaps take the maximum feature value.
#[wasm_bindgen]
pub fn rasterize_geojson(
    geojson_str: String, nrows: usize, ncols: usize,
    xmin: f64, ymax: f64, cellsize: f64, buffer: f64,
) -> Result<Vec<f64>, String> {
    geospatial::rasterize_values(&geojson_str, nrows, ncols,
        &geospatial::GeoTransform { xmin, ymax, cellsize }, buffer)
}

#[wasm_bindgen]
pub fn rasterize_geojson_f32(
    geojson_str: String, nrows: usize, ncols: usize,
    xmin: f64, ymax: f64, cellsize: f64, buffer: f64,
) -> Result<Vec<f32>, String> {
    geospatial::rasterize_values(&geojson_str, nrows, ncols,
        &geospatial::GeoTransform { xmin, ymax, cellsize }, buffer)
}
