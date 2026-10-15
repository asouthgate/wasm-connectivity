// Compute the linear resistance for each pixel based on the distance rasters and ranking values
//
// Linear resistance is computed as a function of the distance from features,
// with a buffer applied to the distance values.
// The resistance is also influenced by the ranking of the features,
// with higher ranked features contributing more to the resistance.
//
// # Arguments
// * distance_rasters: a vector of tuples
// * ncols: the number of columns in the distance rasters
// * buffer: the buffer distance (metres) to apply to the distance values
// * rankmax: the maximum rank value for features
// * resmax: the maximum resistance value
// * xmax: the exponent for the resistance calculation
// * pixw: the width of a pixel in metres (used to convert the buffer to cells)
// # Returns
// A 2D array of f64 values representing the linear resistance for each pixel
pub fn get_linear_resistance(
    distance_rasters: &[(Vec<f64>, f64)],
    missing: &[bool],
    nrows: usize,
    ncols: usize,
    buffer: f64,
    rankmax: f64,
    resmax: f64,
    xmax: f64,
    pixw: f64,
) -> Vec<f64> {
    let total = nrows * ncols;
    let mut resistance = initialize_linear_resistance((0..total).map(|i| missing[i]));
    for (dist, ranking) in distance_rasters {
        update_linear_resistance(&mut resistance, dist, *ranking, buffer, rankmax, resmax, xmax, pixw);
    }
    resistance
}

pub(crate) fn initialize_linear_resistance(missing: impl Iterator<Item = bool>) -> Vec<f64> {
    missing.map(|missing| if missing { f64::NAN } else { 1.0 }).collect()
}

pub(crate) fn update_linear_resistance(
    resistance: &mut [f64],
    dist: &[f64],
    ranking: f64,
    buffer: f64,
    rankmax: f64,
    resmax: f64,
    xmax: f64,
    pixw: f64,
) {
    // Distances are in pixels, while the buffer is in metres.
    let buffer_cells = buffer / pixw;
    let rbuff = ((0.5 + 0.5 * (ranking / rankmax)).powf(xmax) * resmax) + 1.0;
    for i in 0..resistance.len() {
        if resistance[i].is_nan() {
            continue;
        }
        let d = dist[i];
        if !d.is_finite() {
            continue;
        }
        let partial = if d > buffer_cells {
            // Preserve the R reference's extra +1 in the far field.
            rbuff + 1.0
        } else {
            ((0.5 * (d / buffer_cells) + 0.5 * (ranking / rankmax)).powf(xmax) * resmax) + 1.0
        };
        if partial > resistance[i] {
            resistance[i] = partial;
        }
    }
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
        let missing = vec![false; 9];
        let result = get_linear_resistance(&rasters, &missing, nrows, ncols, 10.0, 4.0, 22000.0, 3.0, 1.0);
        assert!(result[0] >= 1.0);
    }

    #[test]
    fn test_multiple_features_max_rule() {
        let nrows = 2;
        let ncols = 2;
        let d1 = vec![0.0, 100.0, 100.0, 100.0];
        let d2 = vec![100.0, 0.0, 100.0, 100.0];
        let rasters = vec![(d1, 4.0), (d2, 1.0)];
        let missing = vec![false; 4];
        let result = get_linear_resistance(&rasters, &missing, nrows, ncols, 10.0, 4.0, 22000.0, 3.0, 1.0);
        assert!(result[1] > result[0], "rank 4 far-field contribution should make cell 1 higher than cell 0");
    }

    #[test]
    fn test_linear_resistance_missing() {
        let nrows = 1;
        let ncols = 3;
        let d1 = vec![0.0, 100.0, 100.0];
        let rasters = vec![(d1, 4.0)];
        let missing = vec![false, true, false];
        let result = get_linear_resistance(&rasters, &missing, nrows, ncols, 10.0, 4.0, 22000.0, 3.0, 1.0);
        assert!(result[0] >= 1.0);
        assert!(result[1].is_nan(), "missing cell should stay NA");
        assert!(result[2].is_finite());
    }
}
