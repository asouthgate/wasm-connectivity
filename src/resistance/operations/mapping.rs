use super::interval::parse_interval_mappings;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Unmatched {
    Number(f64),
    Named(UnmatchedAction),
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnmatchedAction {
    Nodata,
    Error,
}

pub(crate) fn apply_discrete_map(
    values: &[f64],
    intervals: &BTreeMap<String, f64>,
    unmatched: &Unmatched,
) -> Result<Vec<f64>, String> {
    let rules = parse_interval_mappings(intervals)?;
    let mut output = Vec::with_capacity(values.len());
    for (index, &value) in values.iter().enumerate() {
        if !value.is_finite() {
            output.push(f64::NAN);
            continue;
        }
        let mapped = match rules.iter().find(|(interval, _)| interval.contains(value)) {
            Some((_, mapped)) => *mapped,
            None => match unmatched {
                Unmatched::Number(value) if value.is_finite() => *value,
                Unmatched::Named(UnmatchedAction::Nodata) => f64::NAN,
                _ => return Err(format!("unmatched value {value} at cell {index}")),
            },
        };
        output.push(mapped);
    }
    Ok(output)
}
