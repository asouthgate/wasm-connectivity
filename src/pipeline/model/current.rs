use super::{
    graph::PipelineGraph,
    schema::{Input, InputEdges, Node, PipelineDataReference},
};
use crate::pipeline::{operations::Operation, CurrentSolver, PipelineData};
use std::collections::{BTreeMap, HashMap};

impl PipelineGraph {
    pub(in crate::pipeline) fn add_current(
        &mut self,
        inputs: &mut HashMap<String, PipelineData>,
        solver: &CurrentSolver<'_>,
    ) {
        // These names cannot be declared in the model JSON.
        let source = "#current_source";
        let ground = "#current_ground";
        let current = "#current";

        for (name, values) in [(source, solver.source), (ground, solver.ground)] {
            self.inputs.insert(name.into(), Input::RasterRanked {});
            inputs.insert(name.into(), PipelineData::raster(values.to_vec()));
        }
        self.nodes.insert(
            current.into(),
            Node {
                operation: Operation::Current {
                    current_solver_settings: solver.settings,
                },
                inputs: BTreeMap::from([
                    ("resistance".into(), InputEdges::One(self.output.clone())),
                    (
                        "source".into(),
                        InputEdges::One(PipelineDataReference::Input {
                            from_input: source.into(),
                        }),
                    ),
                    (
                        "ground".into(),
                        InputEdges::One(PipelineDataReference::Input {
                            from_input: ground.into(),
                        }),
                    ),
                ]),
                sampling: None,
            },
        );
        // All dependencies were already in the model.
        self.order.push(current.into());
        self.current_output = Some(PipelineDataReference::node(current.into()));
    }
}
