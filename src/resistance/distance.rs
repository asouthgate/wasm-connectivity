// Compute the euclidean distance from nearest feature
//
// This implementation is based on the algorithm described in
// "Distance Transforms of Sampled Functions" by Pedro F. Felzenszwalb and Daniel P. Huttenlocher, 2004.
// https://cs.brown.edu/~pfelzens/papers/dt-final.pdf
// The algorithm computes the distance transform in O(n) time, where n is the number of pixels in the image.
// The algorithm is based on the observation that the distance transform can be computed by first 
// computing the distance transform in one dimension (rows),
// and then computing the distance transform in the other dimension (columns).
//
// 
// # Arguments
// * mask: a 2D array of f64 values, where non-zero values indicate the presence of a feature.
// * nrows: the number of rows in the mask
// * ncols: the number of columns in the mask
// 
// # Returns
// A 2D array of f64 values, where each value is the euclidean distance from the nearest feature in the mask
//  If a pixel is not reachable from any feature, the distance will be NaN.
pub fn euclidean_distance_transform(mask: &[f64], nrows: usize, ncols: usize) -> Vec<f64> {
    let total = nrows * ncols;
    assert_eq!(mask.len(), total);

    let mut f = vec![0.0f64; total];

    // pass 1: compute the distance transform in the row direction
    for r in 0..nrows {
        let row_start = r * ncols;

        let mut nearest: f64 = f64::INFINITY;
        // iterate over cols in first pass to compute the squared distance in the row direction
        for c in 0..ncols {
            let idx = row_start + c;
            if mask[idx] != 0.0 && mask[idx].is_finite() {
                nearest = c as f64;
            }
            if nearest.is_finite() {
                let diff = c as f64 - nearest;
                // in the first pass we only compute the squared distance in the row direction
                // so we store the squared distance in f
                f[idx] = diff * diff;
            } else {
                f[idx] = f64::INFINITY;
            }
        }

        nearest = f64::INFINITY;
        // iterate over cols in reverse order to compute the squared distance in the row direction
        for c in (0..ncols).rev() {
            let idx = row_start + c;
            if mask[idx] != 0.0 && mask[idx].is_finite() {
                nearest = c as f64;
            }
            if nearest.is_finite() {
                let diff = c as f64 - nearest;
                let sq = diff * diff;
                if sq < f[idx] {
                    f[idx] = sq;
                }
            }
        }
    }

    let mut result = vec![0.0f64; total];
    let mut v = vec![0usize; nrows];
    let mut z = vec![0.0f64; nrows + 1];

    // pass 2: compute the distance transform in the column direction
    for c in 0..ncols {
        // Find the first valid row in the column, invalid means the value is NaN or infinite
        let mut first_valid: Option<usize> = None;
        for r in 0..nrows {
            if f[r * ncols + c].is_finite() {
                first_valid = Some(r);
                break;
            }
        }

        // If there is no valid row in the column, set the result to NaN for all rows in that column
        let start_row = match first_valid {
            Some(sr) => sr,
            None => {
                for r in 0..nrows {
                    result[r * ncols + c] = f64::NAN;
                }
                continue;
            }
        };

        let mut k: isize = 0;
        v[0] = start_row;
        z[0] = f64::NEG_INFINITY;
        z[1] = f64::INFINITY;

        // Iterate over the rows in the column, starting from the first valid row
        for r in (start_row + 1)..nrows {
            let fr = f[r * ncols + c];
            if !fr.is_finite() {
                continue;
            }

            loop {
                if k < 0 {
                    break;
                }
                // The important part: compute the intersection of the parabolas
                // defined by the current row and the last row in v
                // The intersection is computed by solving the equation:
                // (r - v[k])^2 + f[v[k]] = (r - v[k+1])^2 + f[v[k+1]]
                // Rearranging the equation gives us the intersection point s:
                // s = ((f[r] + r^2) - (f[v[k]] + v[k]^2)) / (2 * r - 2 * v[k])
                // If the intersection point s is greater than z[k],
                // then we have found a new valid row, and recall a
                // valid row is one that is closer to the current row than the last valid row in v.
                let fv = f[v[k as usize] * ncols + c];
                let r_f = r as f64;
                let v_f = v[k as usize] as f64;
                let denom = 2.0 * r_f - 2.0 * v_f;
                if denom == 0.0 {
                    break;
                }
                let s = ((fr + r_f * r_f) - (fv + v_f * v_f)) / denom;
                if s > z[k as usize] {
                    k += 1;
                    v[k as usize] = r;
                    z[k as usize] = s;
                    z[(k + 1) as usize] = f64::INFINITY;
                    break;
                }
                k -= 1;
            }

            if k < 0 {
                k = 0;
                v[0] = r;
                z[0] = f64::NEG_INFINITY;
                z[1] = f64::INFINITY;
            }
        }

        let mut k2: isize = 0;
        // Iterate over the rows in the column again to compute the final distance transform values
        for r in 0..nrows {
            while k2 < k && z[(k2 + 1) as usize] < r as f64 {
                k2 += 1;
            }
            let vk_f = v[k2 as usize] as f64;
            let diff = r as f64 - vk_f;
            result[r * ncols + c] = diff * diff + f[v[k2 as usize] * ncols + c];
        }
    }

    for val in result.iter_mut() {
        if val.is_finite() && *val >= 0.0 {
            *val = val.sqrt();
        } else {
            *val = f64::NAN;
        }
    }

    result
}

// Compute the euclidean distance from nearest feature, with a buffer applied
//
// # Arguments
// * mask: a 2D array of f64 values, where non-zero values indicate the presence of a feature.
// * nrows: the number of rows in the mask
// * ncols: the number of columns in the mask
// * buffer_cells: the number of cells to buffer the distance by
// # Returns
// A 2D array of f64 values, where each value is the euclidean distance from the nearest feature in the mask, minus the buffer_cells value.
// If a pixel is not reachable from any feature, the distance will be NaN.
pub fn distance_transform_with_buffer(
    mask: &[f64],
    nrows: usize,
    ncols: usize,
    buffer_cells: f64,
) -> Vec<f64> {
    let dist = euclidean_distance_transform(mask, nrows, ncols);
    dist.iter()
        .map(|&d| {
            if d.is_finite() {
                (d - buffer_cells).max(0.0)
            } else {
                d
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_center() {
        let nrows = 5;
        let ncols = 5;
        let mut mask = vec![0.0f64; 25];
        mask[2 * ncols + 2] = 1.0;
        let result = euclidean_distance_transform(&mask, nrows, ncols);

        assert!((result[2 * ncols + 2] - 0.0).abs() < 0.01);
        assert!((result[2 * ncols + 1] - 1.0).abs() < 0.01);
        assert!((result[2 * ncols + 3] - 1.0).abs() < 0.01);
        assert!((result[1 * ncols + 2] - 1.0).abs() < 0.01);
        assert!((result[3 * ncols + 2] - 1.0).abs() < 0.01);
        assert!((result[1 * ncols + 1] - 1.414).abs() < 0.01);
        assert!((result[0 * ncols + 0] - 2.828).abs() < 0.01);
    }

    #[test]
    fn test_no_features() {
        let mask = vec![0.0f64; 9];
        let result = euclidean_distance_transform(&mask, 3, 3);
        for v in result {
            assert!(v.is_nan());
        }
    }

    #[test]
    fn test_all_features() {
        let mask = vec![1.0f64; 9];
        let result = euclidean_distance_transform(&mask, 3, 3);
        for v in result {
            assert!((v - 0.0).abs() < 0.01);
        }
    }
}
