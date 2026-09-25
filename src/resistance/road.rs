use super::distance::euclidean_distance_transform;

// Cal road resistance from binary road raster, a buffer distance, and resistance parameters
// # Arguments
// * road_binary: a 2D array of f64 values where non-zero values indicate the presence of a road
// * nrows: the number of rows in the road raster
// * ncols: the number of columns in the road raster
// * buffer: the buffer distance to apply to the road distance values
// * resmax: the maximum resistance value
// * xmax: the exponent for the resistance calculation
// # Returns
// A 2D array of f64 values representing the road resistance for each pixel
pub fn cal_road_resistance(
    road_binary: &[f64],
    nrows: usize,
    ncols: usize,
    buffer: f64,
    resmax: f64,
    xmax: f64,
) -> Vec<f64> {
    let has_roads = road_binary.iter().any(|&v| v != 0.0 && v.is_finite());

    if !has_roads {
        // No road features: valid (empty) cells have zero road resistance;
        // missing (NaN) cells stay NaN.
        return road_binary
            .iter()
            .map(|&v| if v.is_finite() { 0.0 } else { f64::NAN })
            .collect();
    }

    let road_distance = euclidean_distance_transform(road_binary, nrows, ncols);

    road_distance
        .iter()
        .enumerate()
        .map(|(i, &d)| {
            if !road_binary[i].is_finite() {
                f64::NAN
            } else if !d.is_finite() || d > buffer {
                0.0
            } else {
                ((1.0 - d / buffer) * 0.5 + 0.5).powf(xmax) * resmax
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_roads() {
        let binary = vec![0.0f64; 25];
        let result = cal_road_resistance(&binary, 5, 5, 200.0, 10.0, 5.0);
        assert_eq!(result, vec![0.0f64; 25]);
    }

    #[test]
    fn test_road_resistance_decay() {
        let nrows = 5;
        let ncols = 5;
        let mut binary = vec![0.0f64; 25];
        binary[2 * ncols + 2] = 1.0;
        let buffer = 5.0;
        let resmax = 10.0;
        let xmax = 5.0;
        let result = cal_road_resistance(&binary, nrows, ncols, buffer, resmax, xmax);

        let road_idx = 2 * ncols + 2;
        assert!(result[road_idx] > 0.0, "at road cell should have resistance");
        let expected = ((1.0 - 0.0 / buffer) * 0.5 + 0.5).powf(xmax) * resmax;
        assert!((result[road_idx] - expected).abs() < 0.01, "at road, d=0 → res={}", expected);
    }

    #[test]
    fn test_road_missing() {
        let nrows = 1;
        let ncols = 4;
        let mut binary = vec![0.0f64; 4];
        binary[0] = 1.0;
        binary[2] = f64::NAN;
        let result = cal_road_resistance(&binary, nrows, ncols, 5.0, 10.0, 5.0);
        assert!(result[0] > 0.0);
        assert!(result[1].is_finite());
        assert!(result[2].is_nan(), "missing road data → NaN");
        assert!(result[3].is_finite());
    }
}
