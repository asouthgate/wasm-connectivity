use crate::pipeline::{
    model::{
        interval::{parse_interval_mappings, Interval},
        schema::*,
    },
    NodeOutputs, PipelineData, Result,
};
use crate::resistance::{
    distance::distance_generation, irradiance::irradiance_run, operations::power_response,
    surface::calc_surfs,
};
use std::collections::BTreeMap;

pub(super) struct OperationEvaluator {
    pub(super) grid: Grid,
}

// Responsible for actually evaluating operations on data
impl OperationEvaluator {
    pub(super) fn apply(
        &self,
        op: &Operation,
        inputs: &BTreeMap<String, Vec<PipelineData>>,
    ) -> Result<NodeOutputs> {
        use Operation::*;
        let field = match op {
            Current {
                current_solver_settings,
            } => {
                let source = get_input(inputs, "source")?;
                let ground = get_input(inputs, "ground")?;
                let resistance = get_input(inputs, "resistance")?;
                let current_solver = crate::pipeline::CurrentSolver::new(
                    source.values()?,
                    ground.values()?,
                    *current_solver_settings,
                );
                PipelineData::raster(current_solver.solve_current(
                    resistance.values()?,
                    self.grid.nrows,
                    self.grid.ncols,
                )?)
            }
            Identity => get_input(inputs, "data")?.clone(),
            Round => PipelineData::raster(
                get_input(inputs, "data")?
                    .values()?
                    .iter()
                    .map(|value| value.round())
                    .collect(),
            ),
            DiscreteMap {
                intervals,
                unmatched,
            } => PipelineData::raster(apply_discrete_map(
                get_input(inputs, "data")?.values()?,
                intervals,
                unmatched,
            )?),
            Distance {
                units: DistanceUnits::Metres,
                when_empty,
            } => PipelineData::raster(distances_in_metres(
                get_input(inputs, "data")?.values()?,
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
                get_input(inputs, "data")?.values()?,
                *input_range,
                *base_range,
                *exponent,
                *scale,
                *offset,
                *above,
            )?),
            Override { interval, value } => {
                let interval = Interval::parse(interval)?;
                let mask = get_input(inputs, "where")?;
                let values = get_input(inputs, "data")?.values()?;
                let output = values
                    .iter()
                    .zip(mask.values()?)
                    .map(|(&original, &mask_value)| {
                        if interval.contains(mask_value) {
                            *value
                        } else {
                            original
                        }
                    })
                    .collect();
                PipelineData::raster(output)
            }
            MaskNa {} => {
                let values = get_input(inputs, "data")?.values()?;
                let masks = inputs["mask"]
                    .iter()
                    .map(PipelineData::values)
                    .collect::<Result<Vec<_>>>()?;
                let output = values
                    .iter()
                    .enumerate()
                    .map(|(index, &value)| {
                        if masks.iter().any(|mask| !mask[index].is_finite()) {
                            f64::NAN
                        } else {
                            value
                        }
                    })
                    .collect();
                PipelineData::raster(output)
            }
            Normalise {
                method: Normalisation::PositiveMaximum,
                when_zero,
            } => PipelineData::raster(normalise_positive_maximum(
                get_input(inputs, "data")?.values()?,
                *when_zero,
            )?),
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
        let topography = get_input(inputs, "topography")?;
        let surface = get_input(inputs, "surface")?;
        let occluders = get_input(inputs, "occluders")?;
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
        let PipelineData::Points(points) = get_input(inputs, "point_sources")? else {
            return Err("irradiance requires raycaster_point_source data".into());
        };
        let topography = get_input(inputs, "topography")?;
        let translucent = get_input(inputs, "translucent_surface")?;
        let opaque = get_input(inputs, "opaque_surface")?;
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

fn get_input<'a>(
    inputs: &'a BTreeMap<String, Vec<PipelineData>>,
    name: &str,
) -> Result<&'a PipelineData> {
    inputs
        .get(name)
        .and_then(|values| values.first())
        .ok_or_else(|| format!("missing input port {name}"))
}

fn apply_discrete_map(
    values: &[f64],
    intervals: &BTreeMap<String, f64>,
    unmatched: &Unmatched,
) -> Result<Vec<f64>> {
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
    values: &[f64],
    [lo, hi]: [f64; 2],
    [start, end]: [f64; 2],
    exponent: f64,
    scale: f64,
    offset: f64,
    above: Option<f64>,
) -> Result<Vec<f64>> {
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

fn normalise_positive_maximum(values: &[f64], when_zero: f64) -> Result<Vec<f64>> {
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

fn aggregate_rasters(fields: &[PipelineData], op: &Operation) -> Result<Vec<f64>> {
    let rasters = fields
        .iter()
        .map(PipelineData::values)
        .collect::<Result<Vec<_>>>()?;
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
            } else if matches!(op, Operation::Sum {}) {
                value + other
            } else {
                value.max(other)
            };
        }
        output.push(if value.is_finite() { value } else { f64::NAN });
    }
    Ok(output)
}
