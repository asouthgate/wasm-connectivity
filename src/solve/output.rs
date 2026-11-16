use super::RasterOutput;
use crate::linalg::real::Real;

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
