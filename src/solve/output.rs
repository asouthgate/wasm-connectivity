use super::{current, GroundMode};
use crate::linalg::real::Real;
use serde::Serialize;

#[derive(Serialize)]
pub struct RasterOutput<F: Real = f64> {
    pub voltages: Vec<F>,
    pub current_map: Vec<F>,
    pub nrows: usize,
    pub ncols: usize,
}

/// Solve output annotated with the total PCG iteration count across all
/// component / pair solves.
#[derive(Serialize)]
pub struct AnnotatedOutput<T: Serialize> {
    pub output: T,
    pub total_iters: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl<F: Real> RasterOutput<F> {
    /// Restore missing resistance cells after solving the filled rectangular grid.
    pub fn mask_invalid_current<R: Real>(
        &mut self,
        resistance: &[R],
        nodata: R,
    ) -> Result<(), String> {
        if self.current_map.len() != resistance.len() {
            return Err("current map dimensions differ from resistance".into());
        }
        if self.voltages.iter().any(|v| !v.is_finite()) {
            return Err("non-finite voltages".into());
        }
        for (value, &resistance) in self.current_map.iter_mut().zip(resistance) {
            if !resistance.is_finite() || resistance <= R::zero() || resistance == nodata {
                *value = F::nan();
            } else if !value.is_finite() {
                return Err("non-finite current".into());
            }
        }
        Ok(())
    }
}

/// Raster output for the full-grid (identity mapping) case: the voltage field
/// is already per-cell, so no node scatter is needed.
pub(super) fn build_raster_output_identity<F: Real>(
    voltage_map: Vec<F>,
    resistance_data: &[F],
    ground_data: &[F],
    nrows: usize,
    ncols: usize,
    nodata: F,
    ground_mode: GroundMode,
) -> RasterOutput::<F> {
    let current_map = current::compute_current_map_from_raster(
        resistance_data,
        ground_data,
        &voltage_map,
        nrows,
        ncols,
        nodata,
        ground_mode == GroundMode::Shunt,
    );
    RasterOutput::<F> {
        voltages: voltage_map,
        current_map,
        nrows,
        ncols,
    }
}

pub(super) fn build_raster_output<F: Real>(
    voltages_global: &[F],
    resistance_data: &[F],
    ground_data: &[F],
    cell_to_node: &[i32],
    nrows: usize,
    ncols: usize,
    nodata: F,
    ground_mode: GroundMode,
) -> RasterOutput::<F> {
    let voltage_map = current::reconstruct_grid_map(voltages_global, cell_to_node, nrows * ncols);
    build_raster_output_identity(
        voltage_map, resistance_data, ground_data, nrows, ncols, nodata, ground_mode,
    )
}
