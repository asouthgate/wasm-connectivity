use super::interval::Interval;

pub(crate) fn round_values(values: &[f64]) -> Vec<f64> {
    values.iter().map(|value| value.round()).collect()
}

pub(crate) fn override_values(
    values: &[f64],
    mask: &[f64],
    interval: &Interval,
    replacement: f64,
) -> Vec<f64> {
    values
        .iter()
        .zip(mask)
        .map(|(&value, &mask_value)| {
            if interval.contains(mask_value) {
                replacement
            } else {
                value
            }
        })
        .collect()
}

pub(crate) fn mask_values(values: &[f64], masks: &[&[f64]]) -> Vec<f64> {
    values
        .iter()
        .enumerate()
        .map(|(index, &value)| {
            if masks.iter().any(|mask| !mask[index].is_finite()) {
                f64::NAN
            } else {
                value
            }
        })
        .collect()
}

pub(crate) fn normalise_positive_maximum(
    values: &[f64],
    when_zero: f64,
) -> Result<Vec<f64>, String> {
    if values
        .iter()
        .any(|value| value.is_infinite() || *value < 0.0)
    {
        return Err("positive maximum normalisation requires nonnegative values".into());
    }
    let maximum = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .fold(0.0, f64::max);
    Ok(values
        .iter()
        .map(|&value| {
            if !value.is_finite() {
                value
            } else if maximum > 0.0 {
                value / maximum
            } else {
                when_zero
            }
        })
        .collect())
}

fn aggregate_rasters(rasters: &[&[f64]], combine: fn(f64, f64) -> f64) -> Result<Vec<f64>, String> {
    let (first, rest) = rasters
        .split_first()
        .ok_or("aggregation requires at least one input")?;
    let mut output = Vec::with_capacity(first.len());
    for (index, &first_value) in first.iter().enumerate() {
        let mut value = first_value;
        for raster in rest {
            let other = raster[index];
            value = if !value.is_finite() || !other.is_finite() {
                f64::NAN
            } else {
                combine(value, other)
            };
        }
        output.push(if value.is_finite() { value } else { f64::NAN });
    }
    Ok(output)
}

pub(crate) fn sum_rasters(rasters: &[&[f64]]) -> Result<Vec<f64>, String> {
    aggregate_rasters(rasters, |value, other| value + other)
}

pub(crate) fn maximum_rasters(rasters: &[&[f64]]) -> Result<Vec<f64>, String> {
    aggregate_rasters(rasters, f64::max)
}
