pub fn get_linear_resistance(
    distance_rasters: &[(Vec<f64>, f64)],
    nrows: usize,
    ncols: usize,
    buffer: f64,
    rankmax: f64,
    resmax: f64,
    xmax: f64,
) -> Vec<f64> {
    let total = nrows * ncols;
    let mut resistance = vec![1.0f64; total];

    for (dist, ranking) in distance_rasters {
        let rbuff = ((0.5 + 0.5 * (ranking / rankmax)).powf(xmax) * resmax) + 1.0;

        for i in 0..total {
            let d = dist[i];
            if !d.is_finite() {
                continue;
            }
            let partial = if d > buffer {
                rbuff
            } else {
                ((0.5 * (d / buffer) + 0.5 * (ranking / rankmax)).powf(xmax) * resmax) + 1.0
            };
            if partial > resistance[i] {
                resistance[i] = partial;
            }
        }
    }

    resistance
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linear_resistance_at_feature() {
        let nrows = 3;
        let ncols = 3;
        let dist = vec![0.0f64; 9];
        let ranking = 4.0f64;
        let rasters = vec![(dist, ranking)];
        let result = get_linear_resistance(&rasters, nrows, ncols, 10.0, 4.0, 22000.0, 3.0);
        assert!(result[0] >= 1.0);
    }

    #[test]
    fn test_multiple_features_max_rule() {
        let nrows = 2;
        let ncols = 2;
        let d1 = vec![0.0, 100.0, 100.0, 100.0];
        let d2 = vec![100.0, 0.0, 100.0, 100.0];
        let rasters = vec![(d1, 4.0), (d2, 1.0)];
        let result = get_linear_resistance(&rasters, nrows, ncols, 10.0, 4.0, 22000.0, 3.0);
        assert!(result[1] > result[0], "rank 4 far-field contribution should make cell 1 higher than cell 0");
    }
}
