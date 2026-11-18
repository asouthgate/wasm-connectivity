use super::output::{json_response, maps_to_f32_reusing_buffers};
use crate::{geospatial, linalg, solve};
use crate::linalg::real::Real;
use wasm_bindgen::prelude::*;

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
        output: maps_to_f32_reusing_buffers(annotated.output, source_data, ground_data),
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
    let maps = maps_to_f32_reusing_buffers(solve::RasterOutput {
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


#[cfg(test)]
mod tests {
    use super::*;

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


}
