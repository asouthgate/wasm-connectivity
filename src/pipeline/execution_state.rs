use super::{
    ExecutionOptions, Grid, InputEdges, NodeOutputs, Operation, PipelineData, PipelineGraph,
    Reference, Result, SampleResult,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Inputs, constant results, and node-use counts reused across samples.
pub(super) struct ExecutionState<'g> {
    pub(super) base: Evaluator<'g>,
    pub(super) varying: BTreeSet<String>,
    pub(super) remaining: HashMap<String, usize>,
    pub(super) layers: BTreeMap<String, NodeOutputs>,
}

impl ExecutionState<'_> {
    pub(super) fn run_sample(
        &self,
        index: usize,
        options: ExecutionOptions,
        current_solver: Option<&crate::pipeline::CurrentSolver<'_>>,
    ) -> Result<SampleResult> {
        let graph = self.base.graph;
        let grid = self.base.grid;
        let mut evaluator = Evaluator {
            graph,
            grid,
            inputs: self.base.inputs.clone(),
            outputs: self.base.outputs.clone(),
        };
        let mut sample_uses = self.remaining.clone();
        let mut layers = self.layers.clone();
        for name in &graph.order {
            if !self.varying.contains(name) {
                continue;
            }
            let node = &graph.nodes[name];
            let operation = node
                .sampling
                .as_ref()
                .map(|s| s.operation(name, index, options.seed))
                .transpose()
                .map_err(|e| format!("sample {index}, node {name}: {e}"))?;
            let output = evaluator
                .evaluate(name, operation.as_ref().unwrap_or(&node.operation))
                .map_err(|e| format!("sample {index}: {e}"))?;
            if options.all_layers && graph.named_nodes.contains(name) {
                layers.insert(name.clone(), output);
            }
            evaluator.release_unused_outputs(name, &mut sample_uses);
        }
        let resistance = match evaluator.resolve(&graph.output)? {
            PipelineData::Raster(values) => values,
            _ => return Err("model output must be a raster".into()),
        };
        // Release intermediates before constructing the current solver hierarchy.
        drop(evaluator);
        let current_map = current_solver
            .map(|solver| solver.solve_current(&resistance, grid.nrows, grid.ncols))
            .transpose()
            .map_err(|e| format!("sample {index}: {e}"))?;
        Ok(SampleResult {
            index,
            resistance,
            current: current_map,
            layers,
        })
    }
}

pub(super) struct Evaluator<'g> {
    pub(super) graph: &'g PipelineGraph,
    pub(super) grid: Grid,
    pub(super) inputs: HashMap<String, PipelineData>,
    pub(super) outputs: HashMap<String, NodeOutputs>,
}
impl Evaluator<'_> {
    pub(super) fn release_unused_outputs(
        &mut self,
        name: &str,
        remaining: &mut HashMap<String, usize>,
    ) {
        for reference in self.graph.nodes[name]
            .inputs
            .values()
            .flat_map(InputEdges::references)
        {
            if let Reference::Node { from_node, .. } = reference {
                let count = remaining.get_mut(from_node).expect("node use count");
                *count -= 1;
                if *count == 0 {
                    self.outputs.remove(from_node);
                }
            }
        }
        if !remaining.contains_key(name) {
            self.outputs.remove(name);
        }
    }

    pub(super) fn resolve(&self, reference: &Reference) -> Result<PipelineData> {
        match reference {
            Reference::Input { from_input } => self
                .inputs
                .get(from_input)
                .cloned()
                .ok_or_else(|| format!("unknown input: {from_input}")),
            Reference::Node { from_node, output } => self
                .outputs
                .get(from_node)
                .ok_or_else(|| format!("node {from_node} has not been calculated"))?
                .get(output.as_deref().unwrap_or("data"))
                .cloned()
                .ok_or_else(|| format!("unknown output on node {from_node}")),
        }
    }
    pub(super) fn node(&mut self, name: &str) -> Result<NodeOutputs> {
        let operation = &self
            .graph
            .nodes
            .get(name)
            .ok_or_else(|| format!("unknown node: {name}"))?
            .operation;
        self.evaluate(name, operation)
    }
    pub(super) fn evaluate(&mut self, name: &str, operation: &Operation) -> Result<NodeOutputs> {
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
            .apply(operation, &inputs)
            .map_err(|error| format!("node {name}: {error}"))?;
        self.outputs.insert(name.into(), outputs.clone());
        Ok(outputs)
    }
}
