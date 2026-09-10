//! Error surface over a grid of candidate roost positions.
//!
//! For each candidate roost position we predict the proportion of calls each detector
//! should have caught using a 2D heat-diffusion kernel (analytic integral over
//! `[t0, t1]` in terms of the exponential integral E1), then compare against
//! the observed proportions. The roost estimate is the grid point with the
//! lowest loss (squared error, normalised to `[0, 1]`).
//!
//! Method: Henley L., Finch D., Mathews F., Jones O., Woolley T. E. (2024),
//! "A simple and fast method for estimating bat roost locations",
//! *Royal Society Open Science* 11(4): 231999.
//! <https://doi.org/10.1098/rsos.231999>

use crate::roost::exp1::exp1;

/// Detector capture radius `r` (metres): the circle around each microphone
/// within which a call is registered. Henley et al. (2024) use `r = 15` m.
///
/// The radius enters the approximated detection probability as `r²/(4Dt)`
/// (their equation 3.9). Because every detector shares the same factor and the
/// method only uses normalised proportions, `r` cancels exactly and does not
/// affect the surface, the best-fit point, or the loss.
const CAPTURE_RADIUS_M: f64 = 15.0;

/// Result of the search: the best (lowest-loss) candidate position.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceResult {
    pub x: f64,
    pub y: f64,
    pub loss: f64,
}

/// Compute the error surface and return the lowest-loss grid point.
///
/// `x`, `y`, `counts` are per-detector arrays of equal length. `on_point` is
/// called with `(x, y, loss)` (raw, un-normalised loss) for every grid point
/// (pass a no-op closure when the full surface is not needed).
///
/// The returned `loss` is the squared-error metric of Henley et al. (2024,
/// equation 4.1) normalised by the maximum loss over the grid, so it lies in
/// `[0, 1]`.
///
/// # Panics
/// Panics if `x`, `y`, `counts` have different lengths, if `x`/`y` are empty,
/// if `grid_size < 2`, or unless `0 < t0 < t1`.
#[allow(clippy::too_many_arguments)]
pub fn compute_error_surface(
    x: &[f64],
    y: &[f64],
    counts: &[f64],
    grid_size: usize,
    diffusivity: f64,
    t0: f64,
    t1: f64,
    mut on_point: impl FnMut(f64, f64, f64),
) -> SurfaceResult {
    assert_eq!(x.len(), y.len(), "x and y must have equal length");
    assert_eq!(x.len(), counts.len(), "counts must match x/y length");
    assert!(!x.is_empty(), "no detectors");
    assert!(grid_size >= 2, "grid_size must be >= 2");
    assert!(t1 > t0 && t0 > 0.0, "require 0 < t0 < t1");

    let total: f64 = counts.iter().sum();
    let data_prop: Vec<f64> = counts.iter().map(|c| c / total).collect();

    let (xmin, xmax) = minmax(x);
    let (ymin, ymax) = minmax(y);

    let prefactor = CAPTURE_RADIUS_M * CAPTURE_RADIUS_M / (4.0 * diffusivity);
    let denom_t1 = 4.0 * diffusivity * t1;
    let denom_t0 = 4.0 * diffusivity * t0;
    let log_ratio = (t1 / t0).ln();

    let n = x.len();
    let mut best = SurfaceResult {
        x: f64::NAN,
        y: f64::NAN,
        loss: f64::INFINITY,
    };
    let mut max_loss = 0.0_f64;

    // Scratch buffer reused across all grid points to avoid per-cell allocation.
    let mut buf = Vec::with_capacity(n);

    // Grid layout matches numpy: `meshgrid(zx, zy).ravel()` -> x varies fastest.
    for iy in 0..grid_size {
        let cy = ymin + (ymax - ymin) * (iy as f64) / ((grid_size - 1) as f64);
        for ix in 0..grid_size {
            let cx = xmin + (xmax - xmin) * (ix as f64) / ((grid_size - 1) as f64);

            let mut detec_sum = 0.0;
            buf.clear();
            for i in 0..n {
                let d2 = (x[i] - cx) * (x[i] - cx) + (y[i] - cy) * (y[i] - cy);
                let detec = if d2 > 0.0 {
                    prefactor
                        * (exp1(d2 / denom_t1) - exp1(d2 / denom_t0))
                } else {
                    prefactor * log_ratio
                };
                detec_sum += detec;
                buf.push(detec);
            }

            // Squared-error loss (Henley et al. 2024, equation 4.1 numerator):
            // more weight on detectors that record more passes.
            let mut loss_acc = 0.0;
            for i in 0..n {
                let d = data_prop[i] - buf[i] / detec_sum;
                loss_acc += d * d;
            }

            on_point(cx, cy, loss_acc);

            if loss_acc > max_loss {
                max_loss = loss_acc;
            }
            if loss_acc < best.loss {
                best.loss = loss_acc;
                best.x = cx;
                best.y = cy;
            }
        }
    }

    // Normalise to rho in [0, 1] by the maximum over the tested grid. This does
    // not change the argmin, but makes the reported value match the paper.
    if max_loss > 0.0 {
        best.loss /= max_loss;
    }

    best
}

fn minmax(v: &[f64]) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for &x in v {
        if x < lo {
            lo = x;
        }
        if x > hi {
            hi = x;
        }
    }
    (lo, hi)
}
