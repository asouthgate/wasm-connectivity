use super::{rasterize_features, GeoTransform, LayerParams};
use crate::linalg::real::Real;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize)]
pub struct LayerMask<F: Real = f64> {
    pub name: String,
    pub data: Vec<F>,
}

#[derive(Serialize)]
pub struct GeospatialOutput<F: Real = f64, V: Real = F> {
    pub resistance_map: Vec<F>,
    pub current_map: Vec<V>,
    pub voltage_map: Vec<V>,
    pub layer_masks: Vec<LayerMask<F>>,
    pub nrows: usize,
    pub ncols: usize,
    pub warnings: Vec<String>,
    /// Total PCG iterations across all component solves.
    pub total_iters: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn prepare_geospatial_layers<F: Real>(
    base_raster: &[F],
    nrows: usize,
    ncols: usize,
    geojson_str: &str,
    layer_params_str: &str,
    xmin: f64,
    ymax: f64,
    cellsize: f64,
) -> (Vec<F>, Vec<LayerMask<F>>, Vec<String>) {
    let layer_params: HashMap<String, LayerParams> = match serde_json::from_str(layer_params_str) {
        Ok(p) => p,
        Err(e) => {
            let mut warnings = vec![format!("layer params parse failed: {}", e)];
            let transform = GeoTransform { xmin, ymax, cellsize };
            let (resistance_data, _m) = rasterize_features(
                geojson_str, &HashMap::new(), base_raster, nrows, ncols, &transform, &mut warnings,
            );
            return (resistance_data, Vec::new(), warnings);
        }
    };

    let transform = GeoTransform { xmin, ymax, cellsize };
    let mut warnings = Vec::new();
    let (resistance_data, m) = rasterize_features(
        geojson_str, &layer_params, base_raster, nrows, ncols, &transform, &mut warnings,
    );
    let mut layer_masks: Vec<LayerMask<F>> = m.into_iter()
        .filter(|(_, v)| v.iter().any(|&x| x > F::zero()))
        .map(|(name, data)| LayerMask { name, data })
        .collect();
    layer_masks.sort_by(|a, b| a.name.cmp(&b.name));
    (resistance_data, layer_masks, warnings)
}

pub fn run_geospatial_pipeline_mg<F: Real>(
    base_raster: &[F],
    nrows: usize,
    ncols: usize,
    nodata: F,
    geojson_str: &str,
    layer_params_str: &str,
    xmin: f64,
    ymax: f64,
    cellsize: f64,
    source_data: &[F],
    ground_data: &[F],
    max_iter: usize,
    tol: F,
    ground_mode: crate::solve::GroundMode,
) -> GeospatialOutput<F> {
    run_geospatial_pipeline_mg_impl(
        base_raster, nrows, ncols, nodata, geojson_str, layer_params_str, xmin, ymax, cellsize,
        source_data, ground_data, max_iter, tol, ground_mode, false,
    )
}

/// Like [`run_geospatial_pipeline_mg`], but solves with the
/// low-memory hierarchy: levels 0 and 1 are matrix-free and all transfers
/// are geometric stencils.
pub fn run_geospatial_pipeline_mg_stencil<F: Real>(
    base_raster: &[F],
    nrows: usize,
    ncols: usize,
    nodata: F,
    geojson_str: &str,
    layer_params_str: &str,
    xmin: f64,
    ymax: f64,
    cellsize: f64,
    source_data: &[F],
    ground_data: &[F],
    max_iter: usize,
    tol: F,
    ground_mode: crate::solve::GroundMode,
) -> GeospatialOutput<F> {
    run_geospatial_pipeline_mg_impl(
        base_raster, nrows, ncols, nodata, geojson_str, layer_params_str, xmin, ymax, cellsize,
        source_data, ground_data, max_iter, tol, ground_mode, true,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_geospatial_pipeline_mg_impl<F: Real>(
    base_raster: &[F],
    nrows: usize,
    ncols: usize,
    nodata: F,
    geojson_str: &str,
    layer_params_str: &str,
    xmin: f64,
    ymax: f64,
    cellsize: f64,
    source_data: &[F],
    ground_data: &[F],
    max_iter: usize,
    tol: F,
    ground_mode: crate::solve::GroundMode,
    use_stencil: bool,
) -> GeospatialOutput<F> {
    let (resistance_data, layer_masks, warnings) = prepare_geospatial_layers(
        base_raster, nrows, ncols, geojson_str, layer_params_str, xmin, ymax, cellsize,
    );

    let annotated = if use_stencil {
        crate::solve::solve_raster_sources_mg_stencil(
            &resistance_data, nrows, ncols, nodata,
            source_data, ground_data, max_iter, tol, true, ground_mode,
        )
    } else {
        crate::solve::solve_raster_sources_mg(
            &resistance_data, nrows, ncols, nodata,
            source_data, ground_data, max_iter, tol, true, ground_mode,
        )
    };

    GeospatialOutput {
        resistance_map: resistance_data,
        current_map: annotated.output.current_map,
        voltage_map: annotated.output.voltages,
        layer_masks,
        nrows: annotated.output.nrows,
        ncols: annotated.output.ncols,
        warnings,
        total_iters: annotated.total_iters,
        error: annotated.error,
    }
}

pub fn run_geospatial_pipeline_mg_mixed(
    base_raster: &[f32], nrows: usize, ncols: usize, nodata: f32,
    geojson_str: &str, layer_params_str: &str, xmin: f64, ymax: f64, cellsize: f64,
    source_data: &mut [f32], ground_data: &[f32], max_iter: usize, tol: f64,
    ground_mode: crate::solve::GroundMode,
) -> GeospatialOutput<f32, f64> {
    let (resistance_data, layer_masks, warnings) = prepare_geospatial_layers(
        base_raster, nrows, ncols, geojson_str, layer_params_str, xmin, ymax, cellsize,
    );
    let annotated = crate::solve::mixed::solve_raster_sources_mg_mixed_with_options(
        &resistance_data, nrows, ncols, nodata, source_data, ground_data, max_iter, tol,
        true, ground_mode, crate::linalg::multigrid::MgOptions::default(),
    );
    GeospatialOutput {
        resistance_map: resistance_data, layer_masks, warnings, nrows, ncols,
        current_map: annotated.output.current_map, voltage_map: annotated.output.voltages,
        total_iters: annotated.total_iters, error: annotated.error,
    }
}
