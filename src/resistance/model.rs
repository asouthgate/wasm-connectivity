use super::{
    distance::distance_generation, irradiance::irradiance_run, operations::power_response,
    surface::calc_surfs,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub name: String,
    pub grid: String,
    pub missing_data: MissingData,
    pub inputs: BTreeMap<String, Input>,
    pub layers: BTreeMap<String, Layer>,
    pub output: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingData {
    Propagate,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    Raster {
        path: PathBuf,
    },
    Vector {
        path: PathBuf,
        value_field: String,
        background: f64,
    },
    Points {
        path: PathBuf,
        height_field: String,
    },
}

#[derive(Debug)]
pub enum Layer {
    Pipeline {
        input: String,
        operations: Vec<Operation>,
    },
    Operation(Operation),
}

impl<'de> Deserialize<'de> for Layer {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("op").is_some() {
            serde_json::from_value(value)
                .map(Self::Operation)
                .map_err(serde::de::Error::custom)
        } else {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Pipeline {
                input: String,
                operations: Vec<Operation>,
            }
            let pipeline: Pipeline =
                serde_json::from_value(value).map_err(serde::de::Error::custom)?;
            Ok(Self::Pipeline {
                input: pipeline.input,
                operations: pipeline.operations,
            })
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Round,
    DiscreteMap {
        intervals: BTreeMap<String, f64>,
        unmatched: Unmatched,
    },
    Distance {
        units: DistanceUnits,
        when_empty: EmptyDistance,
    },
    PowerResponse {
        input_range: [f64; 2],
        base_range: [f64; 2],
        exponent: f64,
        scale: f64,
        offset: f64,
        above: Option<f64>,
    },
    Override {
        r#where: Condition,
        value: f64,
    },
    Mask {
        require_finite: Vec<String>,
    },
    Normalise {
        method: Normalisation,
        when_zero: f64,
    },
    SurfaceHeights {
        terrain: String,
        surface: String,
        buildings: String,
    },
    Maximum {
        inputs: Vec<String>,
    },
    Sum {
        inputs: Vec<String>,
    },
    Irradiance {
        lamps: String,
        terrain: String,
        soft_surface: String,
        hard_surface: String,
        cutoff_metres: f64,
        sensor_height_metres: f64,
        absorption: f64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub input: String,
    pub interval: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistanceUnits {
    Metres,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Normalisation {
    PositiveMaximum,
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum EmptyDistance {
    Number(f64),
    Named(Infinity),
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Infinity {
    Infinity,
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Unmatched {
    Number(f64),
    Named(UnmatchedAction),
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnmatchedAction {
    Nodata,
    Error,
}

#[derive(Clone, Debug)]
pub enum Field {
    Raster(Arc<Vec<f64>>),
    Points(Arc<Vec<f64>>),
    Surfaces {
        soft: Arc<Vec<f64>>,
        hard: Arc<Vec<f64>>,
    },
}
impl Field {
    pub fn raster(values: Vec<f64>) -> Self {
        Self::Raster(Arc::new(values))
    }
    fn values(&self) -> Result<&[f64]> {
        match self {
            Self::Raster(values) => Ok(values),
            _ => Err("expected a raster input".into()),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Grid {
    pub nrows: usize,
    pub ncols: usize,
    pub pixel_width: f64,
}

struct Interval {
    min: f64,
    max: f64,
    include_min: bool,
    include_max: bool,
}
impl Interval {
    fn parse(text: &str) -> Result<Self> {
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
    fn contains(&self, v: f64) -> bool {
        v.is_finite()
            && (v > self.min || (self.include_min && v == self.min))
            && (v < self.max || (self.include_max && v == self.max))
    }
    fn overlaps(&self, other: &Self) -> bool {
        !(self.max < other.min
            || other.max < self.min
            || (self.max == other.min && !(self.include_max && other.include_min))
            || (other.max == self.min && !(other.include_max && self.include_min)))
    }
}

impl Model {
    /// Evaluate named layers independent of declaration order; retain shared fields once.
    pub fn run(&self, inputs: HashMap<String, Field>, grid: Grid) -> Result<Vec<f64>> {
        if grid.nrows == 0
            || grid.ncols == 0
            || !grid.pixel_width.is_finite()
            || grid.pixel_width <= 0.0
            || grid.nrows.checked_mul(grid.ncols).is_none()
        {
            return Err("invalid model grid".into());
        }
        for name in self.inputs.keys().chain(self.layers.keys()) {
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(format!("invalid input/layer name: {name}"));
            }
        }
        if !matches!(self.inputs.get(&self.grid), Some(Input::Raster { .. })) {
            return Err("grid must name a raster input".into());
        }
        for (name, spec) in &self.inputs {
            if self.layers.contains_key(name) {
                return Err(format!("input/layer name collision: {name}"));
            }
            let field = inputs
                .get(name)
                .ok_or_else(|| format!("missing input: {name}"))?;
            match (spec, field) {
                (Input::Raster { .. } | Input::Vector { .. }, Field::Raster(values))
                    if values.len() == grid.nrows * grid.ncols => {}
                (Input::Points { .. }, Field::Points(values)) if values.len() % 3 == 0 => {
                    for point in values.chunks_exact(3) {
                        if !point.iter().all(|v| v.is_finite())
                            || point[0] < 0.0
                            || point[1] < 0.0
                            || point[0].round() >= grid.ncols as f64
                            || point[1].round() >= grid.nrows as f64
                            || point[2] < 0.0
                        {
                            return Err(format!(
                                "invalid point coordinates or height in input {name}"
                            ));
                        }
                    }
                }
                _ => return Err(format!("wrong input type or dimensions: {name}")),
            }
        }
        if inputs.keys().any(|name| !self.inputs.contains_key(name)) {
            return Err("undeclared model input supplied".into());
        }
        let mut evaluator = Evaluator {
            model: self,
            grid,
            fields: inputs,
            active: HashSet::new(),
        };
        for name in self.layers.keys() {
            evaluator.resolve(name)?;
        }
        Ok(evaluator.resolve(&self.output)?.values()?.to_vec())
    }
}

struct Evaluator<'a> {
    model: &'a Model,
    grid: Grid,
    fields: HashMap<String, Field>,
    active: HashSet<String>,
}
impl Evaluator<'_> {
    fn resolve(&mut self, name: &str) -> Result<Field> {
        if let Some((parent, component)) = name.split_once('.') {
            return match (self.resolve(parent)?, component) {
                (Field::Surfaces { soft, .. }, "soft") => Ok(Field::Raster(soft)),
                (Field::Surfaces { hard, .. }, "hard") => Ok(Field::Raster(hard)),
                _ => Err(format!("unknown raster output: {name}")),
            };
        }
        if let Some(field) = self.fields.get(name) {
            return Ok(field.clone());
        }
        let layer = self
            .model
            .layers
            .get(name)
            .ok_or_else(|| format!("unknown input/layer: {name}"))?;
        if !self.active.insert(name.into()) {
            return Err(format!("cyclic layer reference: {name}"));
        }
        let result = (|| match layer {
            Layer::Operation(operation) => self.apply(operation, None),
            Layer::Pipeline { input, operations } => {
                let mut field = self.resolve(input)?;
                for (index, operation) in operations.iter().enumerate() {
                    field = self
                        .apply(operation, Some(field))
                        .map_err(|e| format!("operation {}: {e}", index + 1))?;
                }
                Ok(field)
            }
        })()
        .map_err(|e| format!("layer {name}: {e}"));
        self.active.remove(name);
        let field = result?;
        self.fields.insert(name.into(), field.clone());
        Ok(field)
    }

    fn apply(&mut self, op: &Operation, current: Option<Field>) -> Result<Field> {
        use Operation::*;
        let unary = !matches!(
            op,
            SurfaceHeights { .. } | Maximum { .. } | Sum { .. } | Irradiance { .. }
        );
        if unary != current.is_some() {
            return Err(if unary {
                "operation requires a pipeline input"
            } else {
                "operation must be a named layer, not a pipeline step"
            }
            .into());
        }
        let raster = || -> Result<Vec<f64>> {
            Ok(current
                .as_ref()
                .ok_or("missing pipeline input")?
                .values()?
                .to_vec())
        };
        match op {
            Round => Ok(Field::raster(
                raster()?.into_iter().map(f64::round).collect(),
            )),
            DiscreteMap {
                intervals,
                unmatched,
            } => {
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
                let mut values = raster()?;
                for (i, value) in values.iter_mut().enumerate() {
                    if !value.is_finite() {
                        *value = f64::NAN;
                        continue;
                    }
                    *value = match rules.iter().find(|(interval, _)| interval.contains(*value)) {
                        Some((_, mapped)) => *mapped,
                        None => match unmatched {
                            Unmatched::Number(v) if v.is_finite() => *v,
                            Unmatched::Named(UnmatchedAction::Nodata) => f64::NAN,
                            _ => return Err(format!("unmatched value {value} at cell {i}")),
                        },
                    };
                }
                Ok(Field::raster(values))
            }
            Distance {
                units: DistanceUnits::Metres,
                when_empty,
            } => {
                let empty = match when_empty {
                    EmptyDistance::Number(v) if v.is_finite() && *v >= 0.0 => {
                        *v / self.grid.pixel_width
                    }
                    EmptyDistance::Named(Infinity::Infinity) => f64::INFINITY,
                    _ => return Err("empty distance must be nonnegative".into()),
                };
                let field = current.as_ref().ok_or("missing distance input")?;
                let mut distances =
                    distance_generation(field.values()?, self.grid.nrows, self.grid.ncols, empty);
                for d in &mut distances {
                    *d *= self.grid.pixel_width;
                }
                Ok(Field::raster(distances))
            }
            PowerResponse {
                input_range: [lo, hi],
                base_range: [start, end],
                exponent,
                scale,
                offset,
                above,
            } => {
                if ![lo, hi, start, end, exponent, scale, offset]
                    .iter()
                    .all(|v| v.is_finite())
                    || hi <= lo
                    || *exponent <= 0.0
                    || *scale < 0.0
                    || above.is_some_and(|v| !v.is_finite())
                {
                    return Err("invalid power response parameters".into());
                }
                let mut values = raster()?;
                for (i, value) in values.iter_mut().enumerate() {
                    if value.is_nan() {
                        continue;
                    }
                    if *value > *hi {
                        *value = above.ok_or_else(|| {
                            format!("value {value} above input range at cell {i}; specify above")
                        })?;
                    } else if !value.is_finite() || *value < *lo {
                        return Err(format!("value {value} below input range at cell {i}"));
                    } else {
                        let base = start + (end - start) * ((*value - lo) / (hi - lo));
                        *value = power_response(base, *exponent, *scale) + offset;
                        if !value.is_finite() {
                            return Err(format!("nonfinite power response at cell {i}"));
                        }
                    }
                }
                Ok(Field::raster(values))
            }
            Override {
                r#where: condition,
                value,
            } => {
                if !value.is_finite() {
                    return Err("override value must be finite".into());
                }
                let interval = Interval::parse(&condition.interval)?;
                let mask = self.resolve(&condition.input)?;
                let mut values = raster()?;
                for (v, &m) in values.iter_mut().zip(mask.values()?) {
                    if interval.contains(m) {
                        *v = *value;
                    }
                }
                Ok(Field::raster(values))
            }
            Mask { require_finite } => {
                if require_finite.is_empty() {
                    return Err("mask requires at least one input".into());
                }
                let mut values = raster()?;
                for name in require_finite {
                    let mask = self.resolve(name)?;
                    for (v, &m) in values.iter_mut().zip(mask.values()?) {
                        if !m.is_finite() {
                            *v = f64::NAN;
                        }
                    }
                }
                Ok(Field::raster(values))
            }
            Normalise {
                method: Normalisation::PositiveMaximum,
                when_zero,
            } => {
                if !when_zero.is_finite() {
                    return Err("when_zero must be finite".into());
                }
                let mut values = raster()?;
                if values.iter().any(|v| v.is_infinite() || *v < 0.0) {
                    return Err("positive maximum normalisation requires nonnegative values".into());
                }
                let max = values
                    .iter()
                    .copied()
                    .filter(|v| v.is_finite())
                    .fold(0.0, f64::max);
                for v in &mut values {
                    if v.is_finite() {
                        *v = if max > 0.0 { *v / max } else { *when_zero };
                    }
                }
                Ok(Field::raster(values))
            }
            SurfaceHeights {
                terrain,
                surface,
                buildings,
            } => {
                let terrain = self.resolve(terrain)?;
                let surface = self.resolve(surface)?;
                let buildings = self.resolve(buildings)?;
                let output = calc_surfs(
                    terrain.values()?,
                    surface.values()?,
                    buildings.values()?,
                    self.grid.nrows,
                    self.grid.ncols,
                );
                Ok(Field::Surfaces {
                    soft: Arc::new(output.soft_surf),
                    hard: Arc::new(output.hard_surf),
                })
            }
            Maximum { inputs } | Sum { inputs } => {
                let (first, rest) = inputs
                    .split_first()
                    .ok_or("aggregation requires at least one input")?;
                let mut values = self.resolve(first)?.values()?.to_vec();
                for name in rest {
                    let field = self.resolve(name)?;
                    for (v, &other) in values.iter_mut().zip(field.values()?) {
                        *v = if !v.is_finite() || !other.is_finite() {
                            f64::NAN
                        } else if matches!(op, Sum { .. }) {
                            *v + other
                        } else {
                            v.max(other)
                        };
                    }
                }
                for v in &mut values {
                    if !v.is_finite() {
                        *v = f64::NAN;
                    }
                }
                Ok(Field::raster(values))
            }
            Irradiance {
                lamps,
                terrain,
                soft_surface,
                hard_surface,
                cutoff_metres,
                sensor_height_metres,
                absorption,
            } => {
                if ![cutoff_metres, sensor_height_metres, absorption]
                    .iter()
                    .all(|v| v.is_finite() && **v >= 0.0)
                {
                    return Err("irradiance parameters must be finite and nonnegative".into());
                }
                let lamps = self.resolve(lamps)?;
                let Field::Points(points) = lamps else {
                    return Err("irradiance lamps must be a points input".into());
                };
                let terrain = self.resolve(terrain)?;
                let soft = self.resolve(soft_surface)?;
                let hard = self.resolve(hard_surface)?;
                Ok(Field::raster(irradiance_run(
                    &points,
                    soft.values()?,
                    hard.values()?,
                    terrain.values()?,
                    self.grid.nrows,
                    self.grid.ncols,
                    self.grid.pixel_width,
                    *cutoff_metres,
                    *sensor_height_metres,
                    *absorption,
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resistance::pipeline::{run_resistance_pipeline, ResistanceParams};
    use serde_json::{json, Value};

    fn example() -> Model {
        serde_json::from_str(include_str!("../../example/horseshoe-bat-model.json")).unwrap()
    }
    fn grid() -> Grid {
        Grid {
            nrows: 1,
            ncols: 6,
            pixel_width: 2.0,
        }
    }
    fn small_model(operations: Value) -> Model {
        serde_json::from_value(json!({
            "name":"test", "grid":"data", "missing_data":"propagate",
            "inputs":{"data":{"type":"raster", "path":"unused.tif"}},
            "layers":{"result":{"input":"data", "operations":operations}}, "output":"result"
        }))
        .unwrap()
    }
    fn run_small(model: &Model, values: Vec<f64>) -> Result<Vec<f64>> {
        model.run(
            HashMap::from([("data".into(), Field::raster(values))]),
            grid(),
        )
    }
    fn assert_same(actual: &[f64], expected: &[f64]) {
        assert_eq!(actual.len(), expected.len());
        for (i, (&a, &b)) in actual.iter().zip(expected).enumerate() {
            assert!(
                a.is_nan() && b.is_nan() || (a - b).abs() <= 1e-12 * b.abs().max(1.0),
                "cell {i}: {a} != {b}"
            );
        }
    }

    #[test]
    fn interval_mapping_respects_endpoints_unmatched_and_missing() {
        let model = small_model(json!([{"op":"discrete_map", "intervals":{
            "(1,3)":4,"(3,6)":1,"[6,inf)":2
        }, "unmatched":0}]));
        assert_same(
            &run_small(&model, vec![1.0, 2.0, 3.0, 4.0, 6.0, f64::NAN]).unwrap(),
            &[0.0, 4.0, 0.0, 1.0, 2.0, f64::NAN],
        );
        for (text, expected) in [
            ("[1,3)", vec![1.0, 2.0]),
            ("(3,5]", vec![4.0, 5.0]),
            ("[4,4]", vec![4.0]),
        ] {
            let interval = Interval::parse(text).unwrap();
            let actual: Vec<f64> = (0..7)
                .map(|v| v as f64)
                .filter(|v| interval.contains(*v))
                .collect();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn invalid_intervals_and_overlaps_are_errors() {
        for text in [
            "[3,1]",
            "(3,3)",
            "[inf,inf]",
            "[-inf,1]",
            "[1,2",
            "[NaN,2]",
            "[1,2,3]",
            "é1,3)",
        ] {
            assert!(Interval::parse(text).is_err(), "{text}");
        }
        for intervals in [json!({"[1,3]":1,"[3,5]":2}), json!({"(0,10)":1,"[2,3]":2})] {
            let model =
                small_model(json!([{"op":"discrete_map","intervals":intervals,"unmatched":0}]));
            let error = run_small(&model, vec![0.0; 6]).unwrap_err();
            assert!(
                error.contains("layer result: operation 1: overlapping interval"),
                "{error}"
            );
        }
    }

    #[test]
    fn unmatched_error_and_nodata_are_distinct() {
        let op = json!({"op":"discrete_map", "intervals":{"[1,1]":2},"unmatched":"error"});
        let model = small_model(json!([op]));
        assert!(run_small(&model, vec![3.0; 6])
            .unwrap_err()
            .contains("unmatched value 3 at cell 0"));
        let model =
            small_model(json!([{"op":"discrete_map", "intervals":{}, "unmatched":"nodata"}]));
        assert!(run_small(&model, vec![3.0; 6])
            .unwrap()
            .iter()
            .all(|v| v.is_nan()));
    }

    #[test]
    fn distance_response_preserves_boundary_empty_and_missing_rules() {
        let model = small_model(json!([
            {"op":"distance", "units":"metres", "when_empty":"infinity"},
            {"op":"power_response", "input_range":[0,4], "base_range":[1,0.5],
                "exponent":2,"scale":16,"offset":0,"above":0}
        ]));
        assert_same(
            &run_small(&model, vec![1.0, 0.0, 0.0, 0.0, f64::NAN, 0.0]).unwrap(),
            &[16.0, 9.0, 4.0, 0.0, f64::NAN, 0.0],
        );
        assert_same(
            &run_small(&model, vec![0.0, 0.0, 0.0, 0.0, f64::NAN, 0.0]).unwrap(),
            &[0.0, 0.0, 0.0, 0.0, f64::NAN, 0.0],
        );
    }

    #[test]
    fn invalid_responses_references_cycles_and_types_are_errors() {
        for response in [
            json!({"op":"power_response","input_range":[0,0],"base_range":[0,1],"exponent":1,"scale":1,"offset":0}),
            json!({"op":"power_response","input_range":[0,1],"base_range":[0,1],"exponent":0,"scale":1,"offset":0}),
            json!({"op":"power_response","input_range":[0,1],"base_range":[0,1],"exponent":1,"scale":1,"offset":0}),
            json!({"op":"power_response","input_range":[0,10],"base_range":[-1,-1],"exponent":0.5,"scale":1,"offset":0}),
        ] {
            assert!(run_small(&small_model(json!([response])), vec![2.0; 6]).is_err());
        }
        let mut model = small_model(json!([]));
        model.layers.insert(
            "result".into(),
            Layer::Pipeline {
                input: "missing".into(),
                operations: vec![],
            },
        );
        assert!(run_small(&model, vec![0.0; 6])
            .unwrap_err()
            .contains("unknown input/layer: missing"));
        model.layers.insert(
            "result".into(),
            Layer::Pipeline {
                input: "result".into(),
                operations: vec![],
            },
        );
        assert!(run_small(&model, vec![0.0; 6])
            .unwrap_err()
            .contains("cyclic layer reference"));
        model.layers.insert(
            "result".into(),
            Layer::Operation(Operation::Sum { inputs: vec![] }),
        );
        assert!(run_small(&model, vec![0.0; 6])
            .unwrap_err()
            .contains("at least one input"));
        model
            .layers
            .insert("result".into(), Layer::Operation(Operation::Round));
        assert!(run_small(&model, vec![0.0; 6])
            .unwrap_err()
            .contains("requires a pipeline input"));
        assert!(model
            .run(
                HashMap::from([("data".into(), Field::raster(vec![0.0; 5]))]),
                grid()
            )
            .is_err());
        model.inputs.insert(
            "data".into(),
            Input::Points {
                path: "unused.json".into(),
                height_field: "value".into(),
            },
        );
        assert!(model
            .run(
                HashMap::from([("data".into(), Field::Points(Arc::new(vec![])))]),
                grid()
            )
            .is_err());
    }

    #[test]
    fn model_schema_rejects_unknown_operations_fields_and_policies() {
        let original: Value =
            serde_json::from_str(include_str!("../../example/horseshoe-bat-model.json")).unwrap();
        for (pointer, value) in [
            ("/missing_data", json!("ignore")),
            ("/layers/road_res/operations/0/op", json!("script")),
            ("/layers/road_res/operations/0/units", json!("feet")),
            ("/layers/road_res/operations/0/when_empty", json!("zero")),
            ("/layers/road_res/operations/1/typo", json!(1)),
        ] {
            let mut value_copy = original.clone();
            if let Some(slot) = value_copy.pointer_mut(pointer) {
                *slot = value;
            } else {
                value_copy["layers"]["road_res"]["operations"][1]["typo"] = value;
            }
            assert!(
                serde_json::from_value::<Model>(value_copy).is_err(),
                "{pointer}"
            );
        }
    }

    #[test]
    fn horseshoe_spec_matches_every_existing_resistance_layer() {
        let (m, n) = (13, 17);
        let total = m * n;
        for pixel_width in [1.0, 2.0, 5.0] {
            for scenario in 0..4 {
                let mut dtm = vec![100.0; total];
                let heights = [0.0, 1.0, 2.0, 3.0, 4.0, 6.0, 8.0];
                let mut dsm: Vec<f64> = (0..total)
                    .map(|i| 100.0 + heights[i % heights.len()])
                    .collect();
                let mut buildings = vec![0.0; total];
                let mut lcm: Vec<f64> = (0..total).map(|i| (i % 23) as f64).collect();
                let mut roads = vec![0.0; total];
                let mut rivers = vec![0.0; total];
                let mut generic = vec![3.0; total];
                let lamps = if scenario == 0 {
                    vec![]
                } else {
                    vec![4.0, 5.0, 8.0, 12.0, 8.0, 5.0]
                };
                if scenario > 0 {
                    roads[0] = 1.0;
                    roads[total - 1] = 2.0;
                    rivers[n + 2] = 1.0;
                }
                if scenario == 2 {
                    roads.fill(1.0);
                    rivers.fill(1.0);
                }
                if scenario == 3 {
                    dsm.fill(102.0);
                }
                buildings[10] = 5.0;
                buildings[11] = 4.0;
                buildings[12] = 2.0;
                dsm[10] = f64::NAN;
                dtm[11] = f64::NAN;
                lcm[12] = f64::NAN;
                dtm[13] = f64::NAN;
                dsm[14] = f64::NAN;
                roads[15] = f64::NAN;
                rivers[16] = f64::NAN;
                generic[17] = f64::NAN;
                let params = ResistanceParams {
                    road_buffer: 200.0,
                    road_resmax: 10.0,
                    road_xmax: 5.0,
                    river_buffer: 10.0,
                    river_resmax: 2000.0,
                    river_xmax: 4.0,
                    landscape_rankmax: 8.0,
                    landscape_resmax: 100.0,
                    landscape_xmax: 5.0,
                    linear_buffer: 10.0,
                    linear_rankmax: 4.0,
                    linear_resmax: 22000.0,
                    linear_xmax: 3.0,
                    lamp_resmax: 1e8,
                    lamp_xmax: 1.0,
                    lamp_ext: 100.0,
                    pixw: pixel_width,
                    nrows: m,
                    ncols: n,
                };
                let expected = run_resistance_pipeline(
                    &roads,
                    &rivers,
                    &buildings,
                    Some(&lcm),
                    &dtm,
                    &dsm,
                    &generic,
                    &lamps,
                    &params,
                    None,
                    None,
                );
                let mut fields = HashMap::new();
                for (name, values) in [
                    ("dtm", dtm),
                    ("dsm", dsm),
                    ("landcover", lcm),
                    ("buildings", buildings),
                    ("roads", roads),
                    ("rivers", rivers),
                    ("generic", generic),
                ] {
                    fields.insert(name.into(), Field::raster(values));
                }
                fields.insert("lamps".into(), Field::Points(Arc::new(lamps)));
                let mut model = example();
                for (name, expected) in [
                    ("road_res", expected.road_res),
                    ("river_res", expected.river_res),
                    ("landscape_res", expected.landscape_res),
                    ("linear_res", expected.linear_res),
                    ("lamp_res", expected.lamp_res),
                    ("total_res", expected.total_res),
                    ("surfaces.soft", expected.soft_surf),
                    ("surfaces.hard", expected.hard_surf),
                ] {
                    model.output = name.into();
                    let actual = model
                        .run(
                            fields.clone(),
                            Grid {
                                nrows: m,
                                ncols: n,
                                pixel_width,
                            },
                        )
                        .unwrap();
                    assert_same(&actual, &expected);
                }
            }
        }
    }
}
