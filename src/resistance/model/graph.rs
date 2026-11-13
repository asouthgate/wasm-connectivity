use super::{
    interval::{parse_interval_mappings, Interval},
    schema::*,
    Result,
};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Raster,
    Topography,
    Surface,
    Ranked,
    Distance,
    Occluder,
    Height,
    Resistance,
    Irradiance,
    Lights,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Metres,
    Dimensionless,
    Resistance,
    RelativeIrradiance,
    None,
}
#[derive(Debug, Clone, Copy)]
struct Descriptor {
    kind: Kind,
    unit: Unit,
}
impl Descriptor {
    fn new(kind: Kind, unit: Unit) -> Self {
        Self { kind, unit }
    }
    fn is_raster(self) -> bool {
        self.kind != Kind::Lights
    }
}
impl Input {
    fn descriptor(&self) -> Descriptor {
        use Kind::*;
        match self {
            Self::RasterTopography {} => Descriptor::new(Topography, Unit::Metres),
            Self::RasterSurface {} => Descriptor::new(Surface, Unit::Metres),
            Self::RasterRanked {} => Descriptor::new(Ranked, Unit::Dimensionless),
            Self::VectorDistance {} => Descriptor::new(Distance, Unit::Metres),
            Self::VectorOccluder {} => Descriptor::new(Occluder, Unit::Metres),
            Self::VectorResistance {} => Descriptor::new(Resistance, Unit::Resistance),
            Self::RaycasterPointSource {} => Descriptor::new(Lights, Unit::None),
        }
    }
}
#[derive(Clone, Copy)]
enum Accept {
    Any,
    Raster,
    Topography,
    Surface,
    Occluder,
    Height,
    Lights,
}
impl Accept {
    fn accepts(self, descriptor: Descriptor) -> bool {
        match self {
            Self::Any => true,
            Self::Raster => descriptor.is_raster(),
            Self::Topography => descriptor.kind == Kind::Topography,
            Self::Surface => descriptor.kind == Kind::Surface,
            Self::Occluder => descriptor.kind == Kind::Occluder,
            Self::Height => matches!(descriptor.kind, Kind::Height | Kind::Occluder),
            Self::Lights => descriptor.kind == Kind::Lights,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Any => "data",
            Self::Raster => "raster",
            Self::Topography => "raster_topography",
            Self::Surface => "raster_surface",
            Self::Occluder => "vector_occluder",
            Self::Height => "height raster in metres",
            Self::Lights => "raycaster_point_source",
        }
    }
}
pub(super) struct Port {
    pub(super) name: &'static str,
    accepts: Accept,
    pub(super) many: bool,
}
impl Operation {
    pub(super) fn ports(&self) -> &'static [Port] {
        use Accept::*;
        match self {
            Self::Identity => &[Port {
                name: "data",
                accepts: Any,
                many: false,
            }],
            Self::Override { .. } => &[
                Port {
                    name: "data",
                    accepts: Raster,
                    many: false,
                },
                Port {
                    name: "where",
                    accepts: Raster,
                    many: false,
                },
            ],
            Self::MaskNa {} => &[
                Port {
                    name: "data",
                    accepts: Raster,
                    many: false,
                },
                Port {
                    name: "mask",
                    accepts: Raster,
                    many: true,
                },
            ],
            Self::Sum {} | Self::Maximum {} => &[Port {
                name: "data",
                accepts: Raster,
                many: true,
            }],
            Self::SurfaceHeights {} => &[
                Port {
                    name: "topography",
                    accepts: Topography,
                    many: false,
                },
                Port {
                    name: "surface",
                    accepts: Surface,
                    many: false,
                },
                Port {
                    name: "occluders",
                    accepts: Occluder,
                    many: false,
                },
            ],
            Self::Irradiance { .. } => &[
                Port {
                    name: "point_sources",
                    accepts: Lights,
                    many: false,
                },
                Port {
                    name: "topography",
                    accepts: Topography,
                    many: false,
                },
                Port {
                    name: "translucent_surface",
                    accepts: Height,
                    many: false,
                },
                Port {
                    name: "opaque_surface",
                    accepts: Height,
                    many: false,
                },
            ],
            _ => &[Port {
                name: "data",
                accepts: Raster,
                many: false,
            }],
        }
    }
    fn output_types(
        &self,
        inputs: &BTreeMap<String, Vec<Descriptor>>,
    ) -> Result<BTreeMap<String, Descriptor>> {
        use Kind::*;
        let data = || inputs["data"][0];
        let descriptor = match self {
            Self::Identity | Self::Round | Self::Override { .. } | Self::MaskNa {} => data(),
            Self::DiscreteMap { .. } => Descriptor::new(Ranked, Unit::Dimensionless),
            Self::Distance { .. } => Descriptor::new(Distance, Unit::Metres),
            Self::PowerResponse { .. } => Descriptor::new(Resistance, Unit::Resistance),
            Self::Normalise { .. } => Descriptor::new(Raster, Unit::Dimensionless),
            Self::Irradiance { .. } => Descriptor::new(Irradiance, Unit::RelativeIrradiance),
            Self::SurfaceHeights {} => {
                return Ok(BTreeMap::from([
                    ("translucent".into(), Descriptor::new(Height, Unit::Metres)),
                    ("opaque".into(), Descriptor::new(Height, Unit::Metres)),
                ]))
            }
            Self::Sum {} | Self::Maximum {} => {
                let first = data();
                if inputs["data"].iter().any(|other| other.unit != first.unit) {
                    return Err("data inputs must have the same units".into());
                }
                Descriptor::new(
                    if inputs["data"].iter().all(|other| other.kind == first.kind) {
                        first.kind
                    } else {
                        Raster
                    },
                    first.unit,
                )
            }
        };
        Ok(BTreeMap::from([("data".into(), descriptor)]))
    }
}

impl Operation {
    fn validate_parameters(&self) -> Result<()> {
        match self {
            Self::DiscreteMap {
                intervals,
                unmatched,
            } => {
                parse_interval_mappings(intervals)?;
                if matches!(unmatched,Unmatched::Number(value) if !value.is_finite()) {
                    return Err("unmatched value must be finite".into());
                }
            }
            Self::Distance {
                when_empty: EmptyDistance::Number(value),
                ..
            } if !value.is_finite() || *value < 0.0 => {
                return Err("empty distance must be nonnegative".into())
            }
            Self::PowerResponse {
                input_range: [lo, hi],
                base_range: [start, end],
                exponent,
                scale,
                offset,
                above,
            } => {
                if ![lo, hi, start, end, exponent, scale, offset]
                    .iter()
                    .all(|value| value.is_finite())
                    || hi <= lo
                    || *exponent <= 0.0
                    || *scale < 0.0
                    || above.is_some_and(|value| !value.is_finite())
                {
                    return Err("invalid power response parameters".into());
                }
            }
            Self::Override { interval, value } => {
                Interval::parse(interval)?;
                if !value.is_finite() {
                    return Err("override value must be finite".into());
                }
            }
            Self::Normalise { when_zero, .. } if !when_zero.is_finite() => {
                return Err("when_zero must be finite".into())
            }
            Self::Irradiance {
                cutoff_metres,
                sensor_height_metres,
                absorption,
            } => {
                if ![cutoff_metres, sensor_height_metres, absorption]
                    .iter()
                    .all(|value| value.is_finite() && **value >= 0.0)
                {
                    return Err("irradiance parameters must be finite and nonnegative".into());
                }
            }
            _ => {}
        }
        Ok(())
    }
}

pub struct Graph<'a> {
    pub(super) model: &'a Model,
    pub(super) nodes: BTreeMap<String, Node>,
}
impl Model {
    pub fn compile(&self) -> Result<Graph<'_>> {
        for name in self.inputs.keys().chain(self.nodes.keys()) {
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(format!("invalid input/node name: {name}"));
            }
        }
        if self.inputs.keys().any(|name| self.nodes.contains_key(name)) {
            return Err("input/node name collision".into());
        }
        let validate_ref = |reference: &Reference| -> Result<()> {
            match reference {
                Reference::Input { from_input } if !self.inputs.contains_key(from_input) => {
                    Err(format!("unknown input: {from_input}"))
                }
                Reference::Node { from_node, .. } if !self.nodes.contains_key(from_node) => {
                    Err(format!("unknown node: {from_node}"))
                }
                _ => Ok(()),
            }
        };
        validate_ref(&self.output)?;
        let mut nodes = BTreeMap::new();
        for (name, spec) in &self.nodes {
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
                                inputs: BTreeMap::from([(
                                    "data".into(),
                                    Connection::One(input.clone()),
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
                        step.inputs.insert("data".into(), Connection::One(previous));
                        let id = if index + 1 == operations.len() {
                            name.clone()
                        } else {
                            format!("{name}#{index}")
                        };
                        previous = Reference::node(id.clone());
                        nodes.insert(id, step);
                    }
                }
            }
        }
        let graph = Graph { model: self, nodes };
        let mut checker = TypeChecker {
            graph: &graph,
            active: HashSet::new(),
            outputs: HashMap::new(),
        };
        for name in graph.nodes.keys() {
            checker.node(name)?;
        }
        if !checker.reference(&self.output)?.is_raster() {
            return Err("model output must be a raster".into());
        }
        Ok(graph)
    }
}

struct TypeChecker<'a, 'm> {
    graph: &'a Graph<'m>,
    active: HashSet<String>,
    outputs: HashMap<String, BTreeMap<String, Descriptor>>,
}
impl TypeChecker<'_, '_> {
    fn reference(&mut self, reference: &Reference) -> Result<Descriptor> {
        match reference {
            Reference::Input { from_input } => self
                .graph
                .model
                .inputs
                .get(from_input)
                .map(Input::descriptor)
                .ok_or_else(|| format!("unknown input: {from_input}")),
            Reference::Node { from_node, output } => self
                .node(from_node)?
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
        if !self.active.insert(name.into()) {
            return Err(format!("cyclic node reference: {name}"));
        }
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
                if port.many != matches!(connection, Connection::Many(_)) {
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
        self.active.remove(name);
        let outputs = result?;
        self.outputs.insert(name.into(), outputs.clone());
        Ok(outputs)
    }
}
