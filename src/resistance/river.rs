use super::distance::euclidean_distance_transform;

pub fn cal_river_resistance(
    river_binary: &[f64],
    nrows: usize,
    ncols: usize,
    buffer: f64,
    resmax: f64,
    xmax: f64,
) -> Vec<f64> {
    let total = nrows * ncols;
    let has_rivers = river_binary.iter().any(|&v| v != 0.0 && v.is_finite());

    if !has_rivers {
        return vec![resmax; total];
    }

    let mut river_distance = euclidean_distance_transform(river_binary, nrows, ncols);

    for v in river_distance.iter_mut() {
        if !v.is_finite() {
            *v = 0.0;
        }
    }

    river_distance
        .iter()
        .map(|&d| {
            if d > buffer {
                resmax
            } else {
                (d / buffer).powf(xmax) * resmax
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_rivers() {
        let result = cal_river_resistance(&vec![0.0f64; 25], 5, 5, 10.0, 2000.0, 4.0);
        assert!(result.iter().all(|&v| (v - 2000.0).abs() < 0.01));
    }

    #[test]
    fn test_river_conductance() {
        let nrows = 5;
        let ncols = 5;
        let mut binary = vec![0.0f64; 25];
        binary[2 * ncols + 2] = 1.0;
        let result = cal_river_resistance(&binary, nrows, ncols, 10.0, 2000.0, 4.0);
        assert!((result[2 * ncols + 2] - 0.0).abs() < 0.01, "in river: zero resistance");
    }
}
