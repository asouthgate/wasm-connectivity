use super::power_response;

pub(crate) fn apply_power_response(
    values: &[f64],
    [lo, hi]: [f64; 2],
    [start, end]: [f64; 2],
    exponent: f64,
    scale: f64,
    offset: f64,
    above: Option<f64>,
) -> Result<Vec<f64>, String> {
    let mut output = Vec::with_capacity(values.len());
    for (index, &value) in values.iter().enumerate() {
        if value.is_nan() {
            output.push(value);
            continue;
        }
        let transformed = if value > hi {
            above.ok_or_else(|| {
                format!("value {value} above input range at cell {index}; specify above")
            })?
        } else if !value.is_finite() || value < lo {
            return Err(format!("value {value} below input range at cell {index}"));
        } else {
            let base = start + (end - start) * ((value - lo) / (hi - lo));
            let transformed = power_response(base, exponent, scale) + offset;
            if !transformed.is_finite() {
                return Err(format!("nonfinite power response at cell {index}"));
            }
            transformed
        };
        output.push(transformed);
    }
    Ok(output)
}
