use super::schema::Operation;
use crate::pipeline::Result;
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Draw {
    Values(Vec<f64>),
    Uniform([f64; 2]),
    LogUniform([f64; 2]),
}

#[derive(Clone, Debug)]
pub struct SampledParameters {
    template: Value,
    draws: Vec<(String, Draw)>,
}
impl SampledParameters {
    pub(in crate::pipeline) fn extract(parameters: &mut Value) -> Result<Option<Self>> {
        let mut draws = Vec::new();
        fn visit(value: &mut Value, path: &str, draws: &mut Vec<(String, Draw)>) -> Result<()> {
            match value {
                Value::Object(object) if object.contains_key("sample") => {
                    if object.len() != 1 {
                        return Err(format!("{path}: sample must be the only field"));
                    }
                    let draw: Draw = serde_json::from_value(object["sample"].clone())
                        .map_err(|e| e.to_string())?;
                    let representative = match &draw {
                        Draw::Values(values) => {
                            if values.is_empty() || values.iter().any(|v| !v.is_finite()) {
                                return Err(format!(
                                    "{path}: sampled values must be nonempty and finite"
                                ));
                            }
                            values[0]
                        }
                        Draw::Uniform([lo, hi]) | Draw::LogUniform([lo, hi]) => {
                            if !lo.is_finite()
                                || !hi.is_finite()
                                || lo > hi
                                || matches!(draw, Draw::LogUniform(_)) && *lo <= 0.0
                            {
                                return Err(format!("{path}: invalid sampling bounds"));
                            }
                            *lo
                        }
                    };
                    draws.push((path.into(), draw));
                    *value = Value::from(representative);
                }
                Value::Object(object) => {
                    for (key, child) in object {
                        let key = key.replace('~', "~0").replace('/', "~1");
                        visit(child, &format!("{path}/{key}"), draws)?;
                    }
                }
                Value::Array(array) => {
                    for (i, child) in array.iter_mut().enumerate() {
                        visit(child, &format!("{path}/{i}"), draws)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        visit(parameters, "", &mut draws)?;
        Ok((!draws.is_empty()).then(|| Self {
            template: parameters.clone(),
            draws,
        }))
    }
    pub(in crate::pipeline) fn validate_count(&self, count: usize) -> Result<()> {
        for (path, draw) in &self.draws {
            if let Draw::Values(values) = draw {
                if values.len() != count {
                    return Err(format!(
                        "{path}: expected {count} sample values, got {}",
                        values.len()
                    ));
                }
            }
        }
        Ok(())
    }
    pub(in crate::pipeline) fn operation(
        &self,
        node: &str,
        index: usize,
        seed: u64,
    ) -> Result<Operation> {
        let mut parameters = self.template.clone();
        for (path, draw) in &self.draws {
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
}
