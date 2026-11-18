use super::schema::*;
use crate::pipeline::{
    operations::{Descriptor, Operation},
    Result,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone)]
pub(crate) struct PipelineGraph {
    pub(in crate::pipeline) _name: String,
    pub(in crate::pipeline) grid: Option<GridSpec>,
    pub(in crate::pipeline) inputs: BTreeMap<String, Input>,
    pub(in crate::pipeline) output: PipelineDataReference,
    pub(in crate::pipeline) current_output: Option<PipelineDataReference>,
    pub(in crate::pipeline) nodes: BTreeMap<String, Node>,
    pub(in crate::pipeline) named_nodes: BTreeSet<String>,
    pub(in crate::pipeline) order: Vec<String>,
}
impl PipelineGraph {
    pub fn from_json(json: &str) -> Result<Self> {
        let definition =
            serde_json::from_str(json).map_err(|error| format!("Invalid model JSON: {error}"))?;
        Self::from_definition(definition)
    }

    pub fn inputs(&self) -> &BTreeMap<String, Input> {
        &self.inputs
    }

    pub(in crate::pipeline) fn from_definition(
        definition: super::schema::JsonPipeline,
    ) -> Result<Self> {
        for name in definition.inputs.keys().chain(definition.nodes.keys()) {
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(format!("invalid input/node name: {name}"));
            }
        }
        if definition
            .inputs
            .keys()
            .any(|name| definition.nodes.contains_key(name))
        {
            return Err("input/node name collision".into());
        }
        let validate_ref = |reference: &PipelineDataReference| -> Result<()> {
            match reference {
                PipelineDataReference::Input { from_input }
                    if !definition.inputs.contains_key(from_input) =>
                {
                    Err(format!("unknown input: {from_input}"))
                }
                PipelineDataReference::Node { from_node, .. }
                    if !definition.nodes.contains_key(from_node) =>
                {
                    Err(format!("unknown node: {from_node}"))
                }
                _ => Ok(()),
            }
        };
        validate_ref(&definition.output)?;
        let mut nodes = BTreeMap::new();
        for (name, spec) in &definition.nodes {
            let definitions: &[Node] = match spec {
                NodeSpec::Node(node) => std::slice::from_ref(node),
                NodeSpec::Chain { input, operations } => {
                    validate_ref(input)?;
                    operations
                }
            };
            for node in definitions {
                for connection in node.inputs.values() {
                    for reference in connection.references() {
                        validate_ref(reference)?;
                    }
                }
            }
            match spec {
                NodeSpec::Node(node) => {
                    nodes.insert(name.clone(), node.clone());
                }
                NodeSpec::Chain { input, operations } => {
                    if operations.is_empty() {
                        nodes.insert(
                            name.clone(),
                            Node {
                                operation: Operation::Identity,
                                sampling: None,
                                inputs: BTreeMap::from([(
                                    "data".into(),
                                    InputEdges::One(input.clone()),
                                )]),
                            },
                        );
                    }
                    let mut previous = input.clone();
                    for (index, step) in operations.iter().enumerate() {
                        let mut step = step.clone();
                        if !step
                            .operation
                            .ports()
                            .iter()
                            .any(|port| port.name == "data" && !port.many)
                        {
                            return Err(format!(
                                "node {name}, step {}: chains require a single data input",
                                index + 1
                            ));
                        }
                        if step.inputs.contains_key("data") {
                            return Err(format!(
                                "node {name}, step {}: data is supplied by the chain",
                                index + 1
                            ));
                        }
                        step.inputs.insert("data".into(), InputEdges::One(previous));
                        let id = if index + 1 == operations.len() {
                            name.clone()
                        } else {
                            format!("{name}#{index}")
                        };
                        previous = PipelineDataReference::node(id.clone());
                        nodes.insert(id, step);
                    }
                }
            }
        }
        let order = topological_order(&nodes)?;
        let graph = PipelineGraph {
            _name: definition.name,
            grid: definition.grid,
            inputs: definition.inputs,
            output: definition.output,
            current_output: None,
            named_nodes: definition.nodes.keys().cloned().collect(),
            nodes,
            order,
        };
        let mut checker = TypeChecker {
            graph: &graph,
            outputs: HashMap::new(),
        };
        for name in &graph.order {
            checker.node(name)?;
        }
        if !checker.reference(&graph.output)?.is_raster() {
            return Err("model output must be a raster".into());
        }
        Ok(graph)
    }
}

struct TypeChecker<'a> {
    graph: &'a PipelineGraph,
    outputs: HashMap<String, BTreeMap<String, Descriptor>>,
}
impl TypeChecker<'_> {
    fn reference(&mut self, reference: &PipelineDataReference) -> Result<Descriptor> {
        match reference {
            PipelineDataReference::Input { from_input } => self
                .graph
                .inputs
                .get(from_input)
                .map(Input::descriptor)
                .ok_or_else(|| format!("unknown input: {from_input}")),
            PipelineDataReference::Node { from_node, output } => self
                .outputs
                .get(from_node)
                .ok_or_else(|| format!("node {from_node} has not been checked"))?
                .get(output.as_deref().unwrap_or("data"))
                .copied()
                .ok_or_else(|| {
                    format!(
                        "node {from_node} has no output {}; select a declared output port",
                        output.as_deref().unwrap_or("data")
                    )
                }),
        }
    }
    fn node(&mut self, name: &str) -> Result<BTreeMap<String, Descriptor>> {
        if let Some(outputs) = self.outputs.get(name) {
            return Ok(outputs.clone());
        }
        let node = self
            .graph
            .nodes
            .get(name)
            .ok_or_else(|| format!("unknown node: {name}"))?;
        let result = (|| {
            node.operation.validate_parameters()?;
            let ports = node.operation.ports();
            for input in node.inputs.keys() {
                if !ports.iter().any(|port| port.name == input) {
                    return Err(format!("unknown input port {input}"));
                }
            }
            let mut inputs = BTreeMap::new();
            for port in ports {
                let connection = node
                    .inputs
                    .get(port.name)
                    .ok_or_else(|| format!("missing input port {}", port.name))?;
                if port.many != matches!(connection, InputEdges::Many(_)) {
                    return Err(format!(
                        "input {} expects {}",
                        port.name,
                        if port.many {
                            "a reference array"
                        } else {
                            "one reference"
                        }
                    ));
                }
                if connection.references().is_empty() {
                    return Err(format!(
                        "input {} requires at least one reference",
                        port.name
                    ));
                }
                let mut descriptors = Vec::new();
                for reference in connection.references() {
                    let descriptor = self.reference(reference)?;
                    if !port.accepts.accepts(descriptor) {
                        return Err(format!(
                            "input {} must be {} (got {:?}, {:?})",
                            port.name,
                            port.accepts.name(),
                            descriptor.kind,
                            descriptor.unit
                        ));
                    }
                    descriptors.push(descriptor);
                }
                inputs.insert(port.name.into(), descriptors);
            }
            node.operation.output_types(&inputs)
        })()
        .map_err(|error| format!("node {name}: {error}"));
        let outputs = result?;
        self.outputs.insert(name.into(), outputs.clone());
        Ok(outputs)
    }
}

fn topological_order(nodes: &BTreeMap<String, Node>) -> Result<Vec<String>> {
    let mut remaining = BTreeMap::new();
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (name, node) in nodes {
        let dependencies: BTreeSet<_> = node
            .inputs
            .values()
            .flat_map(InputEdges::references)
            .filter_map(|reference| match reference {
                PipelineDataReference::Node { from_node, .. } => Some(from_node.as_str()),
                _ => None,
            })
            .collect();
        remaining.insert(name.as_str(), dependencies.len());
        for dependency in dependencies {
            dependents.entry(dependency).or_default().push(name);
        }
    }
    let mut ready: BTreeSet<_> = remaining
        .iter()
        .filter_map(|(&name, &count)| (count == 0).then_some(name))
        .collect();
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(name) = ready.pop_first() {
        order.push(name.to_owned());
        if let Some(children) = dependents.get(name) {
            for child in children {
                let count = remaining.get_mut(child).ok_or("unknown dependent node")?;
                *count -= 1;
                if *count == 0 {
                    ready.insert(child);
                }
            }
        }
    }
    if order.len() != nodes.len() {
        return Err("cyclic node reference".into());
    }
    Ok(order)
}
