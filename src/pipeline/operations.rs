use super::{
    execution_state::Evaluator,
    interval::{parse_interval_mappings, Interval},
    schema::*,
    NodeOutputs, PipelineData, Result,
};
use crate::resistance::{
    distance::distance_generation, irradiance::irradiance_run, operations::power_response,
    surface::calc_surfs,
};
use std::collections::BTreeMap;

impl Evaluator<'_> {
    pub(super) fn apply(
        &self,
        op: &Operation,
        inputs: &BTreeMap<String, Vec<PipelineData>>,
    ) -> Result<NodeOutputs> {
        use Operation::*;
        let one = |name| input_data(inputs, name);
        let raster = || -> Result<Vec<f64>> { Ok(one("data")?.values()?.to_vec()) };
        let field = match op {
            Identity => one("data")?.clone(),
            Round => PipelineData::raster(raster()?.into_iter().map(f64::round).collect()),
            DiscreteMap {
                intervals,
                unmatched,
            } => PipelineData::raster(apply_discrete_map(raster()?, intervals, unmatched)?),
            Distance {
                units: DistanceUnits::Metres,
                when_empty,
            } => PipelineData::raster(distances_in_metres(
                one("data")?.values()?,
                self.grid,
                when_empty,
            )?),
            PowerResponse {
                input_range,
                base_range,
                exponent,
                scale,
                offset,
                above,
            } => PipelineData::raster(apply_power_response(
                raster()?,
                *input_range,
                *base_range,
                *exponent,
                *scale,
                *offset,
                *above,
            )?),
            Override { interval, value } => {
                let interval = Interval::parse(interval)?;
                let mask = one("where")?;
                let mut values = raster()?;
                for (v, &m) in values.iter_mut().zip(mask.values()?) {
                    if interval.contains(m) {
                        *v = *value;
                    }
                }
                PipelineData::raster(values)
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
                PipelineData::raster(values)
            }
            Normalise {
                method: Normalisation::PositiveMaximum,
                when_zero,
            } => PipelineData::raster(normalise_positive_maximum(raster()?, *when_zero)?),
            SurfaceHeights {} => return self.surface_heights(inputs),
            Maximum {} | Sum {} => PipelineData::raster(aggregate_rasters(&inputs["data"], op)?),
            Irradiance {
                cutoff_metres,
                sensor_height_metres,
                absorption,
            } => PipelineData::raster(self.irradiance(
                inputs,
                *cutoff_metres,
                *sensor_height_metres,
                *absorption,
            )?),
        };
        Ok(BTreeMap::from([("data".into(), field)]))
    }

    fn surface_heights(&self, inputs: &BTreeMap<String, Vec<PipelineData>>) -> Result<NodeOutputs> {
        let topography = input_data(inputs, "topography")?;
        let surface = input_data(inputs, "surface")?;
        let occluders = input_data(inputs, "occluders")?;
        let output = calc_surfs(
            topography.values()?,
            surface.values()?,
            occluders.values()?,
            self.grid.nrows,
            self.grid.ncols,
        );
        Ok(BTreeMap::from([
            ("translucent".into(), PipelineData::raster(output.soft_surf)),
            ("opaque".into(), PipelineData::raster(output.hard_surf)),
        ]))
    }

    fn irradiance(
        &self,
        inputs: &BTreeMap<String, Vec<PipelineData>>,
        cutoff_metres: f64,
        sensor_height_metres: f64,
        absorption: f64,
    ) -> Result<Vec<f64>> {
        let PipelineData::Points(points) = input_data(inputs, "point_sources")? else {
            return Err("irradiance requires raycaster_point_source data".into());
        };
        let topography = input_data(inputs, "topography")?;
        let translucent = input_data(inputs, "translucent_surface")?;
        let opaque = input_data(inputs, "opaque_surface")?;
        Ok(irradiance_run(
            points,
            translucent.values()?,
            opaque.values()?,
            topography.values()?,
            self.grid.nrows,
            self.grid.ncols,
            self.grid.pixel_width,
            cutoff_metres,
            sensor_height_metres,
            absorption,
        ))
    }
}

fn input_data<'a>(
    inputs: &'a BTreeMap<String, Vec<PipelineData>>,
    name: &str,
) -> Result<&'a PipelineData> {
    inputs
        .get(name)
        .and_then(|values| values.first())
        .ok_or_else(|| format!("missing input port {name}"))
}

fn apply_discrete_map(
    mut values: Vec<f64>,
    intervals: &BTreeMap<String, f64>,
    unmatched: &Unmatched,
) -> Result<Vec<f64>> {
    let rules = parse_interval_mappings(intervals)?;
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
    Ok(values)
}

fn distances_in_metres(values: &[f64], grid: Grid, when_empty: &EmptyDistance) -> Result<Vec<f64>> {
    let empty = match when_empty {
        EmptyDistance::Number(v) if v.is_finite() && *v >= 0.0 => *v / grid.pixel_width,
        EmptyDistance::Named(Infinity::Infinity) => f64::INFINITY,
        _ => return Err("empty distance must be nonnegative".into()),
    };
    let mut distances = distance_generation(values, grid.nrows, grid.ncols, empty);
    for distance in &mut distances {
        *distance *= grid.pixel_width;
    }
    Ok(distances)
}

fn apply_power_response(
    mut values: Vec<f64>,
    [lo, hi]: [f64; 2],
    [start, end]: [f64; 2],
    exponent: f64,
    scale: f64,
    offset: f64,
    above: Option<f64>,
) -> Result<Vec<f64>> {
    for (i, value) in values.iter_mut().enumerate() {
        if value.is_nan() {
            continue;
        }
        if *value > hi {
            *value = above.ok_or_else(|| {
                format!("value {value} above input range at cell {i}; specify above")
            })?;
        } else if !value.is_finite() || *value < lo {
            return Err(format!("value {value} below input range at cell {i}"));
        } else {
            let base = start + (end - start) * ((*value - lo) / (hi - lo));
            *value = power_response(base, exponent, scale) + offset;
            if !value.is_finite() {
                return Err(format!("nonfinite power response at cell {i}"));
            }
        }
    }
    Ok(values)
}

fn normalise_positive_maximum(mut values: Vec<f64>, when_zero: f64) -> Result<Vec<f64>> {
    if values.iter().any(|v| v.is_infinite() || *v < 0.0) {
        return Err("positive maximum normalisation requires nonnegative values".into());
    }
    let max = values
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(0.0, f64::max);
    for value in &mut values {
        if value.is_finite() {
            *value = if max > 0.0 { *value / max } else { when_zero };
        }
    }
    Ok(values)
}

fn aggregate_rasters(fields: &[PipelineData], op: &Operation) -> Result<Vec<f64>> {
    let (first, rest) = fields
        .split_first()
        .ok_or("aggregation requires at least one input")?;
    let mut values = first.values()?.to_vec();
    for field in rest {
        for (value, &other) in values.iter_mut().zip(field.values()?) {
            *value = if !value.is_finite() || !other.is_finite() {
                f64::NAN
            } else if matches!(op, Operation::Sum {}) {
                *value + other
            } else {
                value.max(other)
            };
        }
    }
    for value in &mut values {
        if !value.is_finite() {
            *value = f64::NAN;
        }
    }
    Ok(values)
}
