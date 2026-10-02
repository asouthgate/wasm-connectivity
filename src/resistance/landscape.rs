// Reclassify the UKCEH Land Cover Map (LCM) into the expert-opinion rank
// scale of Finch et al. (2020, Table 1). The rank (1..=8) is the ``Rank``
// term in R = (Rank / Vmax)^x * Rmax, where rank 1 is the most permeable
// (Orchards) and rank 8 the least (Buildings).
//
// The eight paper features and their ranks are:
//   1 Orchards, 2 Deciduous woodland, 3 Scrub, 4 Grassland,
//   5 Coniferous woodland, 6 Arable land, 7 Lake, 8 Buildings.
//
// UKCEH LCM is a categorical 1..=21 code, so it must be mapped onto those
// eight ranks. Classes with no direct paper equivalent (heathland, bog, rock,
// coastal) are mapped to the closest feature; see `ukceh_to_rank` below.
//
// # Arguments
// * lcm: a 2D array of f64 values representing the UKCEH land cover class (1..=21)
// # Returns
// A 2D array of f64 values representing the paper rank for each pixel (NaN where invalid)
pub fn compute_base_rank(lcm: &[f64]) -> Vec<f64> {
    lcm.iter()
        .map(|&v| {
            if !v.is_finite() {
                return f64::NAN;
            }
            let class = v.round() as i32;
            ukceh_to_rank(class).unwrap_or(f64::NAN)
        })
        .collect()
}

// Map a single UKCEH LCM class (1..=21) to the paper's land-cover rank
// (1..=8). Returns None for out-of-range classes.
fn ukceh_to_rank(class: i32) -> Option<f64> {
    Some(match class {
        1 => 2.0, // Broadleaved woodland      -> Deciduous woodland
        2 => 5.0, // Coniferous woodland       -> Coniferous woodland
        3 => 6.0, // Arable and horticulture   -> Arable land
        4 | 5 | 6 | 7 => 4.0, // Improved/Neutral/Calcareous/Acid grassland -> Grassland
        8 => 4.0, // Fen, marsh and swamp      -> Grassland (wet grassland)
        9 | 10 => 3.0, // Heather / Heather grassland -> Scrub (heathland)
        11 => 4.0, // Bog                      -> Grassland (wet, semi-natural)
        12 => 8.0, // Inland rock              -> Buildings (bare/impermeable)
        13 | 14 => 7.0, // Saltwater / Freshwater    -> Lake (water)
        15 | 16 | 17 | 18 => 7.0, // Supralittoral/Littoral rock/sediment -> Lake (coastal)
        19 => 4.0, // Saltmarsh                -> Grassland (vegetated coastal marsh)
        20 | 21 => 8.0, // Urban / Suburban           -> Buildings
        _ => return None,
    })
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
            // R reference docstring: "Resistance in interval [0, resmax]".
            // Ranks at or above rankmax (e.g. buildings, set to rankmax)
            // collapse to resmax so the value range stays bounded.
            if c >= rankmax {
                resmax
            } else {
                (c / rankmax).powf(xmax) * resmax
            }
        })
        .collect()
}

// Force pixels classified as buildings to the highest rank so they collapse
// to Rmax in `ranked_resistance` (Buildings = rank 8, the least permeable).
//
// # Arguments
// * conductance: a mutable 2D array of f64 values representing the rank for each pixel
// * buildings: a 2D array of f64 values where non-zero values indicate the presence of a building
// * rankmax: the maximum rank value (Vmax); buildings are set to this rank
fn apply_building_max(conductance: &mut [f64], buildings: &[f64], rankmax: f64) {
    for i in 0..conductance.len() {
        if conductance[i].is_finite() && buildings[i].is_finite() && buildings[i] > 0.0 {
            conductance[i] = rankmax;
        }
    }
}

// Compute the landscape resistance based on the land cover map (lcm) and buildings
//
// # Arguments
// * lcm: a 2D array of f64 values representing the land cover map (UKCEH classes)
// * buildings: a 2D array of f64 values where non-zero values indicate the presence of a building
// * rankmax: the maximum rank value (Vmax)
// * resmax: the maximum resistance value
// * xmax: the exponent for the resistance calculation
// # Returns
// A 2D array of f64 values representing the landscape resistance for each pixel
pub fn get_landscape_resistance_lcm(
    lcm: &[f64],
    buildings: &[f64],
    rankmax: f64,
    resmax: f64,
    xmax: f64,
) -> Vec<f64> {
    let mut conductance = compute_base_rank(lcm);
    apply_building_max(&mut conductance, buildings, rankmax);
    ranked_resistance(&conductance, rankmax, resmax, xmax)
}

// Compute the landscape resistance based on the base rank, buildings, and resistance parameters
//
// # Arguments
// * base_conductance: a 2D array of f64 values representing the land-cover rank for each pixel
// * buildings: a 2D array of f64 values where non-zero values indicate the presence of a building
// * rankmax: the maximum rank value (Vmax)
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
    apply_building_max(&mut conductance, buildings, rankmax);
    ranked_resistance(&conductance, rankmax, resmax, xmax)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_landscape_basic() {
        // Improved grassland (rank 4): (4/8)^5 * 100 = 3.125
        let lcm = vec![4.0; 9];
        let buildings = vec![0.0; 9];
        let result = get_landscape_resistance_lcm(&lcm, &buildings, 8.0, 100.0, 5.0);
        assert!(result[0] >= 0.0);
        assert!(result[0] <= 100.0);
        assert!((result[0] - 3.125).abs() < 1e-6);
    }

    #[test]
    fn test_building_max_rank() {
        let lcm = vec![4.0; 4];
        let buildings = vec![1.0, 0.0, 0.0, 0.0];
        let result = get_landscape_resistance_lcm(&lcm, &buildings, 8.0, 100.0, 5.0);
        assert!(result[0] > 0.0, "building cell should have non-zero resistance");
        assert!(result[0] > result[1], "building cell should have higher resistance than non-building");
        assert!((result[0] - 100.0).abs() < 1e-6, "building cell should map to Rmax");
    }

    #[test]
    fn test_ukceh_reclassification() {
        // Broadleaved woodland -> 2, Coniferous woodland -> 5, Arable -> 6,
        // Improved grassland -> 4, Freshwater -> 7, Urban -> 8.
        let lcm = vec![1.0, 2.0, 3.0, 4.0, 14.0, 20.0];
        let ranks = compute_base_rank(&lcm);
        assert_eq!(ranks, vec![2.0, 5.0, 6.0, 4.0, 7.0, 8.0]);
    }

    #[test]
    fn test_ukceh_invalid_class_nan() {
        let lcm = vec![0.0, -1.0, 22.0, f64::NAN, 4.0];
        let ranks = compute_base_rank(&lcm);
        assert!(ranks[0].is_nan(), "class 0 is not a valid UKCEH class");
        assert!(ranks[1].is_nan(), "negative class is invalid");
        assert!(ranks[2].is_nan(), "class > 21 is invalid");
        assert!(ranks[3].is_nan(), "NaN input stays NaN");
        assert_eq!(ranks[4], 4.0, "valid class still reclassifies");
    }
}
