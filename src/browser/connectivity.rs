use super::output::{json_response, maps_to_f32_reusing_buffers};
use crate::current::{self, linalg};
use crate::real::Real;
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
    let ground_mode = if use_dirichlet_ground { current::GroundMode::Dirichlet } else { current::GroundMode::Shunt };
    let annotated = current::solve_raster_sources_mg(
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
    let ground_mode = if use_dirichlet_ground { current::GroundMode::Dirichlet } else { current::GroundMode::Shunt };
    let annotated = current::mixed::solve_raster_sources_mg_mixed_with_options(
        &resistance_data, nrows, ncols, nodata, &mut source_data, &ground_data,
        max_iter, tol, true, ground_mode, linalg::multigrid::MgOptions::default(),
    );
    let annotated = current::AnnotatedOutput {
        output: maps_to_f32_reusing_buffers(annotated.output, source_data, ground_data),
        total_iters: annotated.total_iters,
        error: annotated.error,
    };
    drop(resistance_data);
    json_response(&annotated)
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
    let ground_mode = if use_dirichlet_ground { current::GroundMode::Dirichlet } else { current::GroundMode::Shunt };
    let mut annotated = current::solve_raster_sources_mg_stencil_in_place_with_options(
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
            let mode = if dirichlet { current::GroundMode::Dirichlet } else { current::GroundMode::Shunt };
            let expected = current::solve_raster_sources_mg_stencil(
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
