use super::{
    graph::PipelineGraph,
    schema::{Grid, Input},
};
use crate::pipeline::{PipelineData, Result};
use std::{collections::HashMap, sync::Arc};

impl PipelineGraph {
    /// Label named arrays as rasters or point coordinates using the pipeline input declarations.
    pub(crate) fn convert_named_arrays_to_pipeline_inputs(
        &self,
        arrays: HashMap<String, Vec<f64>>,
    ) -> Result<HashMap<String, PipelineData>> {
        let mut inputs = HashMap::new();
        for (name, values) in arrays {
            let declaration = self
                .inputs()
                .get(&name)
                .ok_or_else(|| format!("undeclared model input supplied: {name}"))?;
            let data = match declaration {
                Input::RaycasterPointSource {} => PipelineData::Points(Arc::new(values)),
                _ => PipelineData::raster(values),
            };
            inputs.insert(name, data);
        }
        Ok(inputs)
    }
}

fn validate_input(name: &str, spec: &Input, field: &PipelineData, grid: Grid) -> Result<()> {
    match (spec, field) {
        (Input::RaycasterPointSource {}, PipelineData::Points(values)) if values.len() % 3 == 0 => {
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
            | Input::VectorMask {}
            | Input::VectorOccluder {}
            | Input::VectorResistance {},
            PipelineData::Raster(values),
        ) if values.len() == grid.nrows * grid.ncols => {}
        _ => return Err(format!("wrong input type or dimensions: {name}")),
    }
    Ok(())
}

impl PipelineGraph {
    fn validate_grid(&self, grid: Grid) -> Result<()> {
        if grid.nrows == 0
            || grid.ncols == 0
            || !grid.pixel_width.is_finite()
            || grid.pixel_width <= 0.0
            || grid.nrows.checked_mul(grid.ncols).is_none()
        {
            return Err("invalid model grid".into());
        }
        if let Some(spec) = &self.grid {
            let (rows, cols) = spec.dimensions()?;
            if (rows, cols) != (grid.nrows, grid.ncols)
                || (spec.cellsize - grid.pixel_width).abs() > spec.cellsize * 1e-9
            {
                return Err("model arrays do not match the explicit grid".into());
            }
        } else if !self.inputs.values().any(Input::is_raster) {
            return Err("model requires at least one raster input or an explicit grid".into());
        }
        Ok(())
    }
    pub(in crate::pipeline) fn prepare_inputs(
        &self,
        mut inputs: HashMap<String, PipelineData>,
        grid: Grid,
    ) -> Result<HashMap<String, PipelineData>> {
        self.validate_grid(grid)?;
        for (name, spec) in &self.inputs {
            let field = inputs
                .get(name)
                .ok_or_else(|| format!("missing input: {name}"))?;
            validate_input(name, spec, field, grid)?;
        }
        if inputs.keys().any(|name| !self.inputs.contains_key(name)) {
            return Err("undeclared model input supplied".into());
        }
        for field in inputs.values_mut() {
            if let PipelineData::Points(points) = field {
                let retained = points
                    .chunks_exact(3)
                    .filter(|point| grid.contains_point(point))
                    .flatten()
                    .copied()
                    .collect();
                *points = Arc::new(retained);
            }
        }
        Ok(inputs)
    }
}
