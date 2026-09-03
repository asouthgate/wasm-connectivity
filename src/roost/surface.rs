//! Error surface over a grid of candidate roost positions.
//!
//! For each candidate roost position we predict the proportion of calls each detector
//! should have caught using a 2D heat-diffusion kernel (analytic integral over
//! `[t0, t1]` in terms of the exponential integral E1), then compare against
//! the observed proportions. The roost estimate is the grid point with the
//! lowest loss.

use crate::roost::exp1::exp1;

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
/// called with `(x, y, loss)` for every grid point (pass a no-op closure when
/// the full surface is not needed).
///
/// # Panics
/// Panics if `x`, `y`, `counts` have different lengths, or if `x`/`y` are
/// empty, or if `loss` is not `"l2"`/`"l1"`.
#[allow(clippy::too_many_arguments)]
pub fn compute_error_surface(
    x: &[f64],
    y: &[f64],
    counts: &[f64],
    grid_size: usize,
    capture_radius: f64,
    diffusivity: f64,
    t0: f64,
    t1: f64,
    loss: &str,
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

    let prefactor = capture_radius * capture_radius / (4.0 * diffusivity);
    let denom_t1 = 4.0 * diffusivity * t1;
    let denom_t0 = 4.0 * diffusivity * t0;
    let log_ratio = (t1 / t0).ln();

    let n = x.len();
    let mut best = SurfaceResult {
        x: f64::NAN,
        y: f64::NAN,
        loss: f64::INFINITY,
    };

    let is_l2 = match loss {
        "l2" => true,
        "l1" => false,
        other => panic!("unknown loss {other:?}, expected \"l2\" or \"l1\""),
    };

    // Grid layout matches numpy: `meshgrid(zx, zy).ravel()` -> x varies fastest.
    for iy in 0..grid_size {
        let cy = ymin + (ymax - ymin) * (iy as f64) / ((grid_size - 1) as f64);
        for ix in 0..grid_size {
            let cx = xmin + (xmax - xmin) * (ix as f64) / ((grid_size - 1) as f64);

            let mut detec_sum = 0.0;
            // Reuse a small scratch buffer to avoid allocation in the hot loop.
            let mut buf = Vec::with_capacity(n);
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

            let mut loss_acc = 0.0;
            if is_l2 {
                for i in 0..n {
                    let d = data_prop[i] - buf[i] / detec_sum;
                    loss_acc += d * d;
                }
            } else {
                for i in 0..n {
                    loss_acc += (data_prop[i] - buf[i] / detec_sum).abs();
                }
            }

            on_point(cx, cy, loss_acc);

            if loss_acc < best.loss {
                best.loss = loss_acc;
                best.x = cx;
                best.y = cy;
            }
        }
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
