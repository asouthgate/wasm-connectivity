use super::{
    graph::Graph,
    interval::{parse_interval_mappings, Interval},
    schema::*,
    Result,
};
use crate::resistance::{
    distance::distance_generation, irradiance::irradiance_run, operations::power_response,
    surface::calc_surfs,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

#[derive(Clone, Debug)]
pub enum PipelineData {
    Raster(Arc<Vec<f64>>),
    Points(Arc<Vec<f64>>),
}
impl PipelineData {
    pub fn raster(values: Vec<f64>) -> Self {
        Self::Raster(Arc::new(values))
    }
    pub(super) fn values(&self) -> Result<&[f64]> {
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

pub type NodeOutputs = BTreeMap<String, PipelineData>;

impl Model {
    pub fn run(&self, inputs: HashMap<String, PipelineData>, grid: Grid) -> Result<Vec<f64>> {
        self.run_with_layers(inputs, grid).map(|(output, _)| output)
    }
    pub fn run_with_layers(
        &self,
        inputs: HashMap<String, PipelineData>,
        grid: Grid,
    ) -> Result<(Vec<f64>, BTreeMap<String, NodeOutputs>)> {
        self.compile()?.run_with_layers(inputs, grid)
    }
}
impl Graph<'_> {
    pub fn run_with_layers(
        &self,
        mut inputs: HashMap<String, PipelineData>,
        grid: Grid,
    ) -> Result<(Vec<f64>, BTreeMap<String, NodeOutputs>)> {
        let model = self.model;
        if grid.nrows == 0
            || grid.ncols == 0
            || !grid.pixel_width.is_finite()
            || grid.pixel_width <= 0.0
            || grid.nrows.checked_mul(grid.ncols).is_none()
        {
            return Err("invalid model grid".into());
        }
        if let Some(spec) = &model.grid {
            let (rows, cols) = spec.dimensions()?;
            if (rows, cols) != (grid.nrows, grid.ncols)
                || (spec.cellsize - grid.pixel_width).abs() > spec.cellsize * 1e-9
            {
                return Err("model arrays do not match the explicit grid".into());
            }
        } else if !model.inputs.values().any(Input::is_raster) {
            return Err("model requires at least one raster input or an explicit grid".into());
        }
        for (name, spec) in &model.inputs {
            let field = inputs
                .get(name)
                .ok_or_else(|| format!("missing input: {name}"))?;
            match (spec, field) {
                (Input::RaycasterPointSource {}, PipelineData::Points(values))
                    if values.len() % 3 == 0 =>
                {
                    for point in values.chunks_exact(3) {
                        if !point.iter().all(|v| v.is_finite()) || point[2] < 0.0 {
                            return Err(format!(
                                "invalid point coordinates or height in input {name}"
                            ));
                        }
                    }
                }
                (
                    Input::RasterTopography {}
                    | Input::RasterSurface {}
                    | Input::RasterRanked {}
                    | Input::VectorDistance {}
                    | Input::VectorOccluder {}
                    | Input::VectorResistance {},
                    PipelineData::Raster(values),
                ) if values.len() == grid.nrows * grid.ncols => {}
                _ => return Err(format!("wrong input type or dimensions: {name}")),
            }
        }
        if inputs.keys().any(|name| !model.inputs.contains_key(name)) {
            return Err("undeclared model input supplied".into());
        }
        for field in inputs.values_mut() {
            if let PipelineData::Points(points) = field {
                let retained = points
                    .chunks_exact(3)
                    .filter(|point| {
                        point[0] >= 0.0
                            && point[1] >= 0.0
                            && point[0] < grid.ncols as f64
                            && point[1] < grid.nrows as f64
                    })
                    .flatten()
                    .copied()
                    .collect();
                *points = Arc::new(retained);
            }
        }
        let mut evaluator = Evaluator {
            graph: self,
            grid,
            inputs,
            outputs: HashMap::new(),
        };
        let mut exports = BTreeMap::new();
        for name in model.nodes.keys() {
            exports.insert(name.clone(), evaluator.node(name)?);
        }
        let output = evaluator.resolve(&model.output)?.values()?.to_vec();
        Ok((output, exports))
    }
}
struct Evaluator<'g, 'm> {
    graph: &'g Graph<'m>,
    grid: Grid,
    inputs: HashMap<String, PipelineData>,
    outputs: HashMap<String, NodeOutputs>,
}
impl Evaluator<'_, '_> {
    fn resolve(&mut self, reference: &Reference) -> Result<PipelineData> {
        match reference {
            Reference::Input { from_input } => self
                .inputs
                .get(from_input)
                .cloned()
                .ok_or_else(|| format!("unknown input: {from_input}")),
            Reference::Node { from_node, output } => self
                .node(from_node)?
                .get(output.as_deref().unwrap_or("data"))
                .cloned()
                .ok_or_else(|| format!("unknown output on node {from_node}")),
        }
    }
    fn node(&mut self, name: &str) -> Result<NodeOutputs> {
        if let Some(outputs) = self.outputs.get(name) {
            return Ok(outputs.clone());
        }
        let node = self
            .graph
            .nodes
            .get(name)
            .ok_or_else(|| format!("unknown node: {name}"))?;
        let mut inputs = BTreeMap::new();
        for (port, connection) in &node.inputs {
            let values = connection
                .references()
                .iter()
                .map(|reference| self.resolve(reference))
                .collect::<Result<Vec<_>>>()?;
            inputs.insert(port.clone(), values);
        }
        let outputs = self
            .apply(&node.operation, &inputs)
            .map_err(|error| format!("node {name}: {error}"))?;
        self.outputs.insert(name.into(), outputs.clone());
        Ok(outputs)
    }
    fn apply(
        &self,
        op: &Operation,
        inputs: &BTreeMap<String, Vec<PipelineData>>,
    ) -> Result<NodeOutputs> {
        use Operation::*;
        let one = |name: &str| -> Result<&PipelineData> {
            inputs
                .get(name)
                .and_then(|values| values.first())
                .ok_or_else(|| format!("missing input port {name}"))
        };
        let raster = || -> Result<Vec<f64>> { Ok(one("data")?.values()?.to_vec()) };
        let field: Result<PipelineData> = match op {
            Identity => Ok(one("data")?.clone()),
            Round => Ok(PipelineData::raster(
                raster()?.into_iter().map(f64::round).collect(),
            )),
            DiscreteMap {
                intervals,
                unmatched,
            } => {
                let rules = parse_interval_mappings(intervals)?;
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
                Ok(PipelineData::raster(values))
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
                let field = one("data")?;
                let mut distances =
                    distance_generation(field.values()?, self.grid.nrows, self.grid.ncols, empty);
                for d in &mut distances {
                    *d *= self.grid.pixel_width;
                }
                Ok(PipelineData::raster(distances))
            }
            PowerResponse {
                input_range: [lo, hi],
                base_range: [start, end],
                exponent,
                scale,
                offset,
                above,
            } => {
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
                Ok(PipelineData::raster(values))
            }
            Override { interval, value } => {
                let interval = Interval::parse(interval)?;
                let mask = one("where")?;
                let mut values = raster()?;
                for (v, &m) in values.iter_mut().zip(mask.values()?) {
                    if interval.contains(m) {
                        *v = *value;
                    }
                }
                Ok(PipelineData::raster(values))
            }
            MaskNa {} => {
                let mut values = raster()?;
                for mask in &inputs["mask"] {
                    for (v, &m) in values.iter_mut().zip(mask.values()?) {
                        if !m.is_finite() {
                            *v = f64::NAN;
                        }
                    }
                }
                Ok(PipelineData::raster(values))
            }
            Normalise {
                method: Normalisation::PositiveMaximum,
                when_zero,
            } => {
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
                Ok(PipelineData::raster(values))
            }
            SurfaceHeights {} => {
                let topography = one("topography")?;
                let surface = one("surface")?;
                let occluders = one("occluders")?;
                let output = calc_surfs(
                    topography.values()?,
                    surface.values()?,
                    occluders.values()?,
                    self.grid.nrows,
                    self.grid.ncols,
                );
                return Ok(BTreeMap::from([
                    ("translucent".into(), PipelineData::raster(output.soft_surf)),
                    ("opaque".into(), PipelineData::raster(output.hard_surf)),
                ]));
            }
            Maximum {} | Sum {} => {
                let fields = &inputs["data"];
                let (first, rest) = fields
                    .split_first()
                    .ok_or("aggregation requires at least one input")?;
                let mut values = first.values()?.to_vec();
                for field in rest {
                    for (v, &other) in values.iter_mut().zip(field.values()?) {
                        *v = if !v.is_finite() || !other.is_finite() {
                            f64::NAN
                        } else if matches!(op, Sum {}) {
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
                Ok(PipelineData::raster(values))
            }
            Irradiance {
                cutoff_metres,
                sensor_height_metres,
                absorption,
            } => {
                let PipelineData::Points(points) = one("point_sources")? else {
                    return Err("irradiance requires raycaster_point_source data".into());
                };
                let topography = one("topography")?;
                let translucent = one("translucent_surface")?;
                let opaque = one("opaque_surface")?;
                Ok(PipelineData::raster(irradiance_run(
                    &points,
                    translucent.values()?,
                    opaque.values()?,
                    topography.values()?,
                    self.grid.nrows,
                    self.grid.ncols,
                    self.grid.pixel_width,
                    *cutoff_metres,
                    *sensor_height_metres,
                    *absorption,
                )))
            }
        };
        Ok(BTreeMap::from([("data".into(), field?)]))
    }
}
