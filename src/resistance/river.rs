use super::distance::euclidean_distance_transform;

// Cal river resistance from a binary river raster,
// a buffer distance, and resistance parameters
// # Arguments
// * river_binary: a 2D array of f64 values where non-zero values indicate the presence of a river
// * nrows: the number of rows in the river raster
// * ncols: the number of columns in the river raster
// * buffer: the buffer distance (metres) to apply to the river distance values
// * resmax: the maximum resistance value
// * xmax: the exponent for the resistance calculation
// * pixw: the width of a pixel in metres (used to convert the buffer to cells)
// # Returns
// A 2D array of f64 values representing the river resistance for each pixel
pub fn cal_river_resistance(
    river_binary: &[f64],
    nrows: usize,
    ncols: usize,
    buffer: f64,
    resmax: f64,
    xmax: f64,
    pixw: f64,
) -> Vec<f64> {
    let has_rivers = river_binary.iter().any(|&v| v != 0.0 && v.is_finite());

    if !has_rivers {
        // No river features: everywhere is far from a river, so river
        // resistance is at its maximum. Missing (NaN) cells stay NaN.
        return river_binary
            .iter()
            .map(|&v| if v.is_finite() { resmax } else { f64::NAN })
            .collect();
    }

    let river_distance = euclidean_distance_transform(river_binary, nrows, ncols);

    // `river_distance` is expressed in pixels; convert the buffer (metres)
    // to pixels so the comparison is dimensionally consistent.
    let buffer_cells = buffer / pixw;

    river_distance
        .iter()
        .enumerate()
        .map(|(i, &d)| {
            if !river_binary[i].is_finite() {
                f64::NAN
            } else if !d.is_finite() || d > buffer_cells {
                resmax
            } else {
                (d / buffer_cells).powf(xmax) * resmax
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_rivers() {
        let result = cal_river_resistance(&vec![0.0f64; 25], 5, 5, 10.0, 2000.0, 4.0, 1.0);
        assert!(result.iter().all(|&v| v == 2000.0));
    }

    #[test]
    fn test_river_conductance() {
        let nrows = 5;
        let ncols = 5;
        let mut binary = vec![0.0f64; 25];
        binary[2 * ncols + 2] = 1.0;
        let result = cal_river_resistance(&binary, nrows, ncols, 10.0, 2000.0, 4.0, 1.0);
        assert!((result[2 * ncols + 2] - 0.0).abs() < 0.01, "in river: zero resistance");
    }

    #[test]
    fn test_river_buffer_in_metres() {
        // pixw = 1 m, buffer = 10 m -> buffer spans 10 cells. Verify the ramp
        // (d/buffer)^xmax * resmax within the buffer and resmax beyond it.
        let nrows = 1;
        let ncols = 14;
        let mut binary = vec![0.0f64; 14];
        binary[0] = 1.0;
        let result = cal_river_resistance(&binary, nrows, ncols, 10.0, 2000.0, 4.0, 1.0);
        assert!((result[0] - 0.0).abs() < 0.01, "in river: zero resistance");
        let expected_d5 = (5.0f64 / 10.0).powf(4.0) * 2000.0;
        assert!((result[5] - expected_d5).abs() < 0.01, "5 m out -> ramp value");
        assert!((result[10] - 2000.0).abs() < 0.01, "at buffer edge -> resmax");
        assert!((result[11] - 2000.0).abs() < 0.01, "beyond buffer -> resmax");
    }

    #[test]
    fn test_river_missing() {
        let nrows = 1;
        let ncols = 4;
        let mut binary = vec![0.0f64; 4];
        binary[0] = 1.0;
        binary[2] = f64::NAN;
        let result = cal_river_resistance(&binary, nrows, ncols, 10.0, 2000.0, 4.0, 1.0);
        assert!(result[0] >= 0.0);
        assert!(result[1].is_finite());
        assert!(result[2].is_nan(), "missing river data → NaN");
        assert!(result[3].is_finite());
    }
}
