// Compute the base conductance for each pixel based on the soft surface height and land cover map (lcm)
// 
// # Arguments
// * soft_surf: a 2D array of f64 values representing the soft surface
// * lcm: a 2D array of f64 values representing the land cover map
// # Returns
// A 2D array of f64 values representing the base conductance for each pixel
pub fn compute_base_conductance(soft_surf: &[f64], lcm: &[f64]) -> Vec<f64> {
    let total = soft_surf.len();
    let mut conductance = vec![0.0f64; total];

    for i in 0..total {
        let h_valid = soft_surf[i].is_finite() && soft_surf[i] >= 0.0;
        let lcm_valid = lcm[i].is_finite() && lcm[i] >= 0.0;
        if h_valid && lcm_valid {
            // R reference: grass [-Inf, 0.5) → 4; scrub [0.5, 2.5) → 3; trees [2.5, Inf] → 3
            let lidar_rank = if soft_surf[i] < 0.5 { 4.0 } else { 3.0 };
            conductance[i] = lidar_rank + lcm[i];
        } else {
            conductance[i] = f64::NAN;
        }
    }

    conductance
}

// Compute the resistance for each pixel based on the conductance, rankmax, resmax, and xmax
//
// # Arguments
// * conductance: a 2D array of f64 values representing the conductance for each pixel
// * rankmax: the maximum rank value for conductance
// * resmax: the maximum resistance value
// * xmax: the exponent for the resistance calculation
// # Returns
// A 2D array of f64 values representing the resistance for each pixel
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

// Apply a maximum resistance value to pixels that are classified as buildings
//
// # Arguments
// * conductance: a mutable 2D array of f64 values representing the conductance for each pixel
// * buildings: a 2D array of f64 values where non-zero values indicate the presence of a building
fn apply_building_max(conductance: &mut [f64], buildings: &[f64]) {
    let max_value = conductance.iter().cloned().fold(0.0_f64, f64::max) + 1.0;
    for i in 0..conductance.len() {
        if buildings[i].is_finite() && buildings[i] > 0.0 {
            conductance[i] = max_value;
        }
    }
}

// Compute the landscape resistance based on the land cover map (lcm), buildings, and soft surface height
//
// # Arguments
// * lcm: a 2D array of f64 values representing the land cover map
// * buildings: a 2D array of f64 values where non-zero values indicate the presence of a building
// * soft_surf: a 2D array of f64 values representing the soft surface
// * nrows: the number of rows in the arrays
// * ncols: the number of columns in the arrays
// * rankmax: the maximum rank value for conductance
// * resmax: the maximum resistance value
// * xmax: the exponent for the resistance calculation
// # Returns
// A 2D array of f64 values representing the landscape resistance for each pixel
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

// Compute the landscape resistance based on the base conductance, buildings, and resistance parameters
//
// # Arguments
// * base_conductance: a 2D array of f64 values representing the base conductance for each pixel
// * buildings: a 2D array of f64 values where non-zero values indicate the presence of a building
// * rankmax: the maximum rank value for conductance
// * resmax: the maximum resistance value
// * xmax: the exponent for the resistance calculation
// # Returns
// A 2D array of f64 values representing the landscape resistance for each pixel
pub fn get_landscape_resistance_from_conductance(
    base_conductance: &[f64],
    buildings: &[f64],
    rankmax: f64,
    resmax: f64,
    xmax: f64,
) -> Vec<f64> {
    let mut conductance: Vec<f64> = base_conductance
        .iter()
        .map(|&c| if c.is_finite() && c >= 0.0 { c } else { f64::NAN })
        .collect();
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
