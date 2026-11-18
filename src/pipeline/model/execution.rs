use super::schema::*;
use crate::pipeline::{ExecutionOptions, NodeResult, PipelineData, PipelineGraph, Result};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

impl PipelineGraph {
    fn validate_execution_options(&self, options: ExecutionOptions) -> Result<()> {
        if options.threads == 0 {
            return Err("execution requires at least one thread".into());
        }
        #[cfg(target_arch = "wasm32")]
        if options.threads != 1 {
            return Err("Wasm execution requires one thread".into());
        }
        if options.samples == 0 {
            return Err("execution requires at least one sample".into());
        }
        if options.statistics && options.samples < 2 {
            return Err("sample variance requires at least two samples".into());
        }
        for (name, node) in &self.nodes {
            if let Some(sampling) = &node.sampling {
                sampling
                    .validate_count(options.samples)
                    .map_err(|e| format!("node {name}: {e}"))?;
            }
        }
        Ok(())
    }

    pub(in crate::pipeline) fn sample_dependent_nodes(&self) -> BTreeSet<String> {
        let mut varying = BTreeSet::new();
        for name in &self.order {
            let node = &self.nodes[name];
            if node.sampling.is_some() || node.inputs.values().flat_map(InputEdges::references)
                .any(|r| matches!(r, PipelineDataReference::Node { from_node, .. } if varying.contains(from_node))) {
                varying.insert(name.clone());
            }
        }
        varying
    }

    pub(super) fn prepare_inputs(
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
    fn with_current_node(
        &self,
        inputs: &mut HashMap<String, PipelineData>,
        current_solver: &crate::pipeline::CurrentSolver<'_>,
    ) -> Self {
        let mut graph = self.clone();
        // These internal names cannot collide with names accepted in model JSON.
        for (name, values) in [
            ("#current_source", current_solver.source),
            ("#current_ground", current_solver.ground),
        ] {
            graph.inputs.insert(name.into(), Input::RasterRanked {});
            inputs.insert(name.into(), PipelineData::raster(values.to_vec()));
        }
        graph.nodes.insert(
            "#current".into(),
            Node {
                operation: Operation::Current {
                    current_solver_settings: current_solver.settings(),
                },
                sampling: None,
                inputs: BTreeMap::from([
                    ("resistance".into(), InputEdges::One(graph.output.clone())),
                    (
                        "source".into(),
                        InputEdges::One(PipelineDataReference::Input {
                            from_input: "#current_source".into(),
                        }),
                    ),
                    (
                        "ground".into(),
                        InputEdges::One(PipelineDataReference::Input {
                            from_input: "#current_ground".into(),
                        }),
                    ),
                ]),
            },
        );
        graph.order.push("#current".into());
        graph
    }

    /// Execute the pipeline.
    ///
    /// - `inputs`: Named external rasters and point arrays; vectors must be rasterized first.
    /// - `grid`: Array dimensions and cell size in metres.
    /// - `options`: Sample count, seed, intermediate outputs, and statistics settings.
    /// - `current_solver`: Optional solver applied to each resistance map.
    /// - `on_node_result`: Receives each completed layer or summary without waiting for other layers.
    ///
    /// Errors stop execution.
    pub fn execute(
        &self,
        inputs: HashMap<String, PipelineData>,
        grid: Grid,
        options: ExecutionOptions,
        current_solver: Option<&crate::pipeline::CurrentSolver<'_>>,
        on_node_result: impl FnMut(NodeResult) -> Result<()>,
    ) -> Result<()> {
        self.validate_execution_options(options)?;
        let mut inputs = self.prepare_inputs(inputs, grid)?;
        if let Some(solver) = current_solver {
            solver.validate(grid.nrows * grid.ncols)?;
        }
        let graph_with_current;
        let (graph, current_output) = if let Some(current_solver) = current_solver {
            graph_with_current = self.with_current_node(&mut inputs, current_solver);
            (
                &graph_with_current,
                Some(PipelineDataReference::node("#current".into())),
            )
        } else {
            (self, None)
        };
        let task_list = crate::pipeline::task_graph::scheduler::build_node_task_list(
            graph,
            inputs,
            options,
            current_output,
        )?;
        crate::pipeline::task_graph::scheduler::execute_node_task_list(
            task_list,
            grid,
            options,
            on_node_result,
        )
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
