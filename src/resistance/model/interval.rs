use super::Result;
use std::collections::BTreeMap;

pub(super) struct Interval {
    min: f64,
    max: f64,
    include_min: bool,
    include_max: bool,
}
impl Interval {
    pub(super) fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        if text.len() < 5 || !text.is_ascii() {
            return Err(format!("invalid interval: {text}"));
        }
        let (first, last) = (text.as_bytes()[0], text.as_bytes()[text.len() - 1]);
        if !matches!(first, b'[' | b'(') || !matches!(last, b']' | b')') {
            return Err(format!("invalid interval: {text}"));
        }
        let (low, high) = text[1..text.len() - 1]
            .split_once(',')
            .ok_or_else(|| format!("invalid interval: {text}"))?;
        let bound = |s: &str| -> Result<f64> {
            match s.trim() {
                "-inf" => Ok(f64::NEG_INFINITY),
                "inf" => Ok(f64::INFINITY),
                number => number
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| format!("invalid interval bound: {s}")),
            }
        };
        let interval = Self {
            min: bound(low)?,
            max: bound(high)?,
            include_min: first == b'[',
            include_max: last == b']',
        };
        if interval.min > interval.max
            || interval.min == f64::INFINITY
            || interval.max == f64::NEG_INFINITY
            || (interval.min == interval.max && !(interval.include_min && interval.include_max))
            || (!interval.min.is_finite() && interval.include_min)
            || (!interval.max.is_finite() && interval.include_max)
        {
            return Err(format!("empty or invalid interval: {text}"));
        }
        Ok(interval)
    }
    pub(super) fn contains(&self, v: f64) -> bool {
        v.is_finite()
            && (v > self.min || (self.include_min && v == self.min))
            && (v < self.max || (self.include_max && v == self.max))
    }
    pub(super) fn overlaps(&self, other: &Self) -> bool {
        !(self.max < other.min
            || other.max < self.min
            || (self.max == other.min && !(self.include_max && other.include_min))
            || (other.max == self.min && !(other.include_max && self.include_min)))
    }
}

pub(super) fn mapping_rules(intervals: &BTreeMap<String, f64>) -> Result<Vec<(Interval, f64)>> {
    let mut rules: Vec<(Interval, f64)> = Vec::new();
    for (text, &value) in intervals {
        let interval = Interval::parse(text)?;
        if !value.is_finite() {
            return Err("mapping value must be finite".into());
        }
        if rules.iter().any(|(other, _)| interval.overlaps(other)) {
            return Err(format!("overlapping interval: {text}"));
        }
        rules.push((interval, value));
    }
    Ok(rules)
}
