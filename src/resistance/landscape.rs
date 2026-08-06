pub fn get_landscape_resistance_lcm(
    lcm: &[f64],
    buildings: &[f64],
    soft_surf: &[f64],
    nrows: usize,
    ncols: usize,
    rankmax: f64,
    resmax: f64,
    xmax: f64,
) -> Vec<f64> {
    let total = nrows * ncols;

    let mut conductance = vec![0.0f64; total];
    let mut max_value = f64::NEG_INFINITY;

    for i in 0..total {
        let h = if soft_surf[i].is_finite() {
            soft_surf[i]
        } else {
            0.0
        };
        let lidar_rank = if h < 0.5 {
            4.0
        } else if h < 2.5 {
            3.0
        } else {
            3.0
        };
        let lcm_val = if lcm[i].is_finite() { lcm[i] } else { 0.0 };
        let cond = lidar_rank + lcm_val;
        conductance[i] = cond;
        if cond > max_value {
            max_value = cond;
        }
    }

    max_value += 1.0;

    for i in 0..total {
        if buildings[i].is_finite() && buildings[i] != 0.0 {
            conductance[i] = max_value;
        }
    }

    let mut resistance = vec![0.0f64; total];
    for i in 0..total {
        if conductance[i] >= rankmax {
            resistance[i] = resmax;
        } else {
            resistance[i] = (conductance[i] / rankmax).powf(xmax) * resmax;
        }
    }

    resistance
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_landscape_basic() {
        let nrows = 3;
        let ncols = 3;
        let lcm = vec![1.0; 9];
        let buildings = vec![0.0; 9];
        let soft_surf = vec![1.0; 9];
        let result = get_landscape_resistance_lcm(
            &lcm, &buildings, &soft_surf, nrows, ncols, 8.0, 100.0, 5.0,
        );
        assert!(result[0] >= 0.0);
        assert!(result[0] <= 100.0);
    }

    #[test]
    fn test_building_max_rank() {
        let nrows = 2;
        let ncols = 2;
        let lcm = vec![0.0; 4];
        let buildings = vec![1.0, 0.0, 0.0, 0.0];
        let soft_surf = vec![0.1; 4];
        let result = get_landscape_resistance_lcm(
            &lcm, &buildings, &soft_surf, nrows, ncols, 8.0, 100.0, 5.0,
        );
        assert!(result[0] > 0.0, "building cell should have non-zero resistance");
        assert!(result[0] > result[1], "building cell should have higher resistance than non-building");
    }
}
