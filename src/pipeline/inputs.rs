use super::{Input, PipelineData, PipelineGraph, Result};
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
