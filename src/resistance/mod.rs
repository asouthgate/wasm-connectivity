pub mod distance;
pub mod road;
pub mod river;
pub mod surface;
pub mod landscape;
pub mod linear;
pub mod irradiance;
pub mod pipeline;

use crate::NODATA_SENTINEL;

/// True when a value represents missing data: either non-finite (NaN/±inf) or
/// the pipeline's nodata sentinel (-9999).
pub fn is_missing(v: f64) -> bool {
    !v.is_finite() || v == NODATA_SENTINEL
}
