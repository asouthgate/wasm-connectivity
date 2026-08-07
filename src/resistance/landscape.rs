pub fn compute_base_conductance(soft_surf: &[f64], lcm: &[f64]) -> Vec<f64> {
    let total = soft_surf.len();
    let mut conductance = vec![0.0f64; total];

    for i in 0..total {
        let h = if soft_surf[i].is_finite() {
            soft_surf[i]
        } else {
            0.0
        };
        // R reference: grass [-Inf, 0.5) → 4; scrub [0.5, 2.5) → 3; trees [2.5, Inf] → 3
        let lidar_rank = if h < 0.5 { 4.0 } else { 3.0 };
        let lcm_val = if lcm[i].is_finite() { lcm[i] } else { 0.0 };
        conductance[i] = lidar_rank + lcm_val;
    }

    conductance
}

fn ranked_resistance(conductance: &[f64], rankmax: f64, resmax: f64, xmax: f64) -> Vec<f64> {
    conductance
        .iter()
        .map(|&c| {
            if c >= rankmax {
                resmax
            } else {
                (c / rankmax).powf(xmax) * resmax
            }
        })
        .collect()
}

fn apply_building_max(conductance: &mut [f64], buildings: &[f64]) {
    let max_value = conductance.iter().cloned().fold(0.0_f64, f64::max) + 1.0;
    for i in 0..conductance.len() {
        if buildings[i].is_finite() && buildings[i] > 0.0 {
            conductance[i] = max_value;
        }
    }
}

pub fn get_landscape_resistance_lcm(
    lcm: &[f64],
    buildings: &[f64],
    soft_surf: &[f64],
    _nrows: usize,
    _ncols: usize,
    rankmax: f64,
    resmax: f64,
    xmax: f64,
) -> Vec<f64> {
    let mut conductance = compute_base_conductance(soft_surf, lcm);
    apply_building_max(&mut conductance, buildings);
    ranked_resistance(&conductance, rankmax, resmax, xmax)
}

pub fn get_landscape_resistance_from_conductance(
    base_conductance: &[f64],
    buildings: &[f64],
    rankmax: f64,
    resmax: f64,
    xmax: f64,
) -> Vec<f64> {
    let mut conductance = base_conductance.to_vec();
    apply_building_max(&mut conductance, buildings);
    ranked_resistance(&conductance, rankmax, resmax, xmax)
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
