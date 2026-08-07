// Compute the linear resistance for each pixel based on the distance rasters and ranking values
//
// Linear resistance is computed as a function of the distance from features,
// with a buffer applied to the distance values.
// The resistance is also influenced by the ranking of the features,
// with higher ranked features contributing more to the resistance.
// The biological meaning of this resistance is that linear features
// such as roads, rivers, or other barriers can impede movement or dispersal
// of organisms across the landscape, and the resistance value quantifies
// the degree of impedance based on distance and feature ranking.
//
// # Arguments
// * distance_rasters: a vector of tuples
// * ncols: the number of columns in the distance rasters
// * buffer: the buffer distance to apply to the distance values
// * rankmax: the maximum rank value for features
// * resmax: the maximum resistance value
// * xmax: the exponent for the resistance calculation
// # Returns
// A 2D array of f64 values representing the linear resistance for each pixel
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
