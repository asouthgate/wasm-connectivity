use crate::pipeline::{operations::*, Grid, NodeOutputs, PipelineData, Result};
use crate::resistance::{
    distance::distances_in_metres,
    irradiance::irradiance_run,
    operations::{
        apply_discrete_map, apply_power_response, interval::Interval, mask_values, maximum_rasters,
        normalise_positive_maximum, override_values, round_values, sum_rasters,
    },
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
            Round => PipelineData::raster(round_values(get_input(inputs, "data")?.values()?)),
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
            } => {
                let when_empty = match when_empty {
                    EmptyDistance::Number(value) => *value,
                    EmptyDistance::Named(Infinity::Infinity) => f64::INFINITY,
                };
                PipelineData::raster(distances_in_metres(
                    get_input(inputs, "data")?.values()?,
                    self.grid.nrows,
                    self.grid.ncols,
                    self.grid.pixel_width,
                    when_empty,
                )?)
            }
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
            Override { interval, value } => PipelineData::raster(override_values(
                get_input(inputs, "data")?.values()?,
                get_input(inputs, "where")?.values()?,
                &Interval::parse(interval)?,
                *value,
            )),
            MaskNa {} => {
                let masks = raster_inputs(&inputs["mask"])?;
                PipelineData::raster(mask_values(get_input(inputs, "data")?.values()?, &masks))
            }
            Normalise {
                method: Normalisation::PositiveMaximum,
                when_zero,
            } => PipelineData::raster(normalise_positive_maximum(
                get_input(inputs, "data")?.values()?,
                *when_zero,
            )?),
            SurfaceHeights {} => return self.surface_heights(inputs),
            Maximum {} => PipelineData::raster(maximum_rasters(&raster_inputs(&inputs["data"])?)?),
            Sum {} => PipelineData::raster(sum_rasters(&raster_inputs(&inputs["data"])?)?),
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

fn raster_inputs(fields: &[PipelineData]) -> Result<Vec<&[f64]>> {
    fields.iter().map(PipelineData::values).collect()
}
