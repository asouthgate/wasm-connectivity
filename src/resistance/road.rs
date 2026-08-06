use super::distance::euclidean_distance_transform;

pub fn cal_road_resistance(
    road_binary: &[f64],
    nrows: usize,
    ncols: usize,
    buffer: f64,
    resmax: f64,
    xmax: f64,
) -> Vec<f64> {
    let total = nrows * ncols;
    let has_roads = road_binary.iter().any(|&v| v != 0.0 && v.is_finite());

    if !has_roads {
        return vec![0.0f64; total];
    }

    let road_distance = euclidean_distance_transform(road_binary, nrows, ncols);

    road_distance
        .iter()
        .map(|&d| {
            if !d.is_finite() || d > buffer {
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
}
