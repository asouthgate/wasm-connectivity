/// Apply a class/attribute ranking rule, preserving missing input cells.
pub fn ranker(values: &[f64], classify: impl Fn(f64) -> f64) -> Vec<f64> {
    values.iter().map(|&value| {
        if value.is_finite() { classify(value) } else { f64::NAN }
    }).collect()
}

/// Scale a normalised distance, rank, or intensity through a power curve.
pub fn power_response(value: f64, exponent: f64, maximum: f64) -> f64 {
    value.powf(exponent) * maximum
}

/// Apply a buffered power response to generated distances in place.
/// Infinite distance denotes an absent feature; NaN denotes missing input.
pub fn distance_from_feature_power(
    distances: &mut [f64],
    buffer_cells: f64,
    maximum: f64,
    exponent: f64,
    outside: f64,
    normalise: impl Fn(f64) -> f64,
) {
    for distance in distances {
        *distance = if distance.is_nan() {
            f64::NAN
        } else if !distance.is_finite() || *distance > buffer_cells {
            outside
        } else {
            power_response(normalise(*distance / buffer_cells), exponent, maximum)
        };
    }
}
