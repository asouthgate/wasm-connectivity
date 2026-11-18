use crate::pipeline::Result;
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::pipeline) enum Draw {
    Values(Vec<f64>),
    Uniform([f64; 2]),
    LogUniform([f64; 2]),
}

#[derive(Clone, Debug)]
pub struct SampledParameters {
    pub(in crate::pipeline) template: Value,
    pub(in crate::pipeline) draws: Vec<(String, Draw)>,
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
}
