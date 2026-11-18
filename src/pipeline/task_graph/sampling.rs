use crate::pipeline::{
    model::parameters::{Draw, SampledParameters},
    operations::Operation,
    Result,
};
use serde_json::Value;

pub(super) fn sample_operation(
    sampled_parameters: &SampledParameters,
    node: &str,
    index: usize,
    seed: u64,
) -> Result<Operation> {
    let mut parameters = sampled_parameters.template.clone();
    for (path, draw) in &sampled_parameters.draws {
        let value = match draw {
            Draw::Values(values) => values[index],
            Draw::Uniform([lo, hi]) | Draw::LogUniform([lo, hi]) => {
                // SplitMix64's mixing function, with a stable key for each parameter
                // and sample. Evaluation order cannot change the random draws.
                fn mix(mut x: u64) -> u64 {
                    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
                    x ^ (x >> 31)
                }
                let hash = node
                    .bytes()
                    .chain([0])
                    .chain(path.bytes())
                    .fold(0xcbf29ce484222325u64, |hash, byte| {
                        (hash ^ byte as u64).wrapping_mul(0x100000001b3)
                    });
                let bits = mix(seed
                    .wrapping_add(hash)
                    .wrapping_add((index as u64).wrapping_mul(0x9e3779b97f4a7c15)));
                let u = (bits >> 11) as f64 / 9007199254740992.0;
                if lo == hi {
                    *lo
                } else if matches!(draw, Draw::LogUniform(_)) {
                    ((1.0 - u) * lo.ln() + u * hi.ln()).exp()
                } else {
                    (1.0 - u) * lo + u * hi
                }
            }
        };
        *parameters
            .pointer_mut(path)
            .ok_or_else(|| format!("unknown sampled parameter: {path}"))? = Value::from(value);
    }
    let operation: Operation = serde_json::from_value(parameters).map_err(|e| e.to_string())?;
    operation.validate_parameters()?;
    Ok(operation)
}
