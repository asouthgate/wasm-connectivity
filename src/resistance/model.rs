use super::{
    distance::distance_generation, irradiance::irradiance_run, operations::power_response,
    surface::calc_surfs,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub name: String,
    pub grid: Option<GridSpec>,
    pub inputs: BTreeMap<String, Input>,
    pub nodes: BTreeMap<String, NodeSpec>,
    pub output: Reference,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridSpec {
    pub crs: String,
    pub bounds: [f64; 4],
    pub cellsize: f64,
}

impl GridSpec {
    pub fn dimensions(&self) -> Result<(usize, usize)> {
        let [xmin, ymin, xmax, ymax] = self.bounds;
        if !self.bounds.iter().all(|v| v.is_finite())
            || !self.cellsize.is_finite()
            || self.cellsize <= 0.0
            || xmax <= xmin
            || ymax <= ymin
        {
            return Err("grid requires finite ordered bounds and a positive cellsize".into());
        }
        let cols = (xmax - xmin) / self.cellsize;
        let rows = (ymax - ymin) / self.cellsize;
        if [rows, cols].iter().any(|v| {
            !v.is_finite() || *v < 1.0 || *v >= usize::MAX as f64 || (v - v.round()).abs() > 1e-7
        }) {
            return Err("grid bounds must span whole cells".into());
        }
        let (rows, cols) = (rows.round() as usize, cols.round() as usize);
        rows.checked_mul(cols).ok_or("grid dimensions overflow")?;
        Ok((rows, cols))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    RasterTopography {},
    RasterSurface {},
    RasterRanked {},
    VectorDistance {},
    VectorOccluder {},
    VectorResistance {},
    RaycasterPointSource {},
}

impl Input {
    pub fn is_raster(&self) -> bool {
        matches!(
            self,
            Self::RasterTopography {} | Self::RasterSurface {} | Self::RasterRanked {}
        )
    }
}

#[derive(Debug, Clone)]
pub enum Reference {
    Input {
        from_input: String,
    },
    Node {
        from_node: String,
        output: Option<String>,
    },
}
impl<'de> Deserialize<'de> for Reference {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let reference = String::deserialize(deserializer)?;
        let parts: Vec<_> = reference.split('.').collect();
        let valid_name = |name: &str| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        };
        match parts.as_slice() {
            ["inputs", name] if valid_name(name) => Ok(Self::Input {
                from_input: (*name).into(),
            }),
            ["nodes", name] if valid_name(name) => Ok(Self::Node {
                from_node: (*name).into(),
                output: None,
            }),
            ["nodes", name, output] if valid_name(name) && valid_name(output) => Ok(Self::Node {
                from_node: (*name).into(),
                output: Some((*output).into()),
            }),
            _ => Err(serde::de::Error::custom(
                "expected inputs.<name>, nodes.<name> or nodes.<name>.<output>",
            )),
        }
    }
}
impl Reference {
    fn node(name: String) -> Self {
        Self::Node {
            from_node: name,
            output: None,
        }
    }
    pub fn selects_node(&self, name: &str, port: &str) -> bool {
        matches!(self, Self::Node { from_node, output } if from_node == name && output.as_deref().unwrap_or("data") == port)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Connection {
    One(Reference),
    Many(Vec<Reference>),
}
impl Connection {
    fn references(&self) -> &[Reference] {
        match self {
            Self::One(value) => std::slice::from_ref(value),
            Self::Many(values) => values,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub operation: Operation,
    pub inputs: BTreeMap<String, Connection>,
}
impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Definition {
            op: String,
            #[serde(default)]
            inputs: Option<serde_json::Value>,
            #[serde(default)]
            parameters: serde_json::Map<String, serde_json::Value>,
        }
        let mut definition = Definition::deserialize(deserializer)?;
        if definition.parameters.contains_key("op") {
            return Err(serde::de::Error::custom(
                "op belongs on the node, not in parameters",
            ));
        }
        definition
            .parameters
            .insert("op".into(), serde_json::Value::String(definition.op));
        let operation: Operation =
            serde_json::from_value(serde_json::Value::Object(definition.parameters))
                .map_err(serde::de::Error::custom)?;
        let inputs = match definition.inputs {
            None => BTreeMap::new(),
            Some(value) if operation.ports().len() == 1 && operation.ports()[0].name == "data" => {
                let connection: Connection =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                BTreeMap::from([("data".into(), connection)])
            }
            Some(value) => serde_json::from_value(value).map_err(serde::de::Error::custom)?,
        };
        Ok(Self { operation, inputs })
    }
}

#[derive(Debug, Clone)]
pub enum NodeSpec {
    Chain {
        input: Reference,
        operations: Vec<Node>,
    },
    Node(Node),
}
impl<'de> Deserialize<'de> for NodeSpec {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("op").is_some() {
            serde_json::from_value(value)
                .map(Self::Node)
                .map_err(serde::de::Error::custom)
        } else {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Chain {
                input: Reference,
                operations: Vec<Node>,
            }
            let chain: Chain = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
            Ok(Self::Chain {
                input: chain.input,
                operations: chain.operations,
            })
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Round,
    DiscreteMap {
        intervals: BTreeMap<String, f64>,
        unmatched: Unmatched,
    },
    Distance {
        units: DistanceUnits,
        when_empty: EmptyDistance,
    },
    PowerResponse {
        input_range: [f64; 2],
        base_range: [f64; 2],
        exponent: f64,
        scale: f64,
        offset: f64,
        above: Option<f64>,
    },
    Override {
        interval: String,
        value: f64,
    },
    MaskNa {},
    Normalise {
        method: Normalisation,
        when_zero: f64,
    },
    SurfaceHeights {},
    Maximum {},
    Sum {},
    Irradiance {
        cutoff_metres: f64,
        sensor_height_metres: f64,
        absorption: f64,
    },
    #[serde(skip)]
    Identity,
}

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
struct Port {
    name: &'static str,
    accepts: Accept,
    many: bool,
}
impl Operation {
    fn ports(&self) -> &'static [Port] {
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

fn mapping_rules(intervals: &BTreeMap<String, f64>) -> Result<Vec<(Interval, f64)>> {
    let mut rules: Vec<(Interval, f64)> = Vec::new();
    for (text, &value) in intervals {
        let interval = Interval::parse(text)?;
        if !value.is_finite() {
            return Err("mapping value must be finite".into());
        }
        if rules.iter().any(|(other, _)| interval.overlaps(other)) {
            return Err(format!("overlapping interval: {text}"));
        }
        rules.push((interval, value));
    }
    Ok(rules)
}
impl Operation {
    fn validate_parameters(&self) -> Result<()> {
        match self {
            Self::DiscreteMap {
                intervals,
                unmatched,
            } => {
                mapping_rules(intervals)?;
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

pub type NodeOutputs = BTreeMap<String, Field>;

pub struct Graph<'a> {
    model: &'a Model,
    nodes: BTreeMap<String, Node>,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistanceUnits {
    Metres,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Normalisation {
    PositiveMaximum,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum EmptyDistance {
    Number(f64),
    Named(Infinity),
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Infinity {
    Infinity,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Unmatched {
    Number(f64),
    Named(UnmatchedAction),
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnmatchedAction {
    Nodata,
    Error,
}

#[derive(Clone, Debug)]
pub enum Field {
    Raster(Arc<Vec<f64>>),
    Points(Arc<Vec<f64>>),
}
impl Field {
    pub fn raster(values: Vec<f64>) -> Self {
        Self::Raster(Arc::new(values))
    }
    fn values(&self) -> Result<&[f64]> {
        match self {
            Self::Raster(values) => Ok(values),
            _ => Err("expected a raster input".into()),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Grid {
    pub nrows: usize,
    pub ncols: usize,
    pub pixel_width: f64,
}

struct Interval {
    min: f64,
    max: f64,
    include_min: bool,
    include_max: bool,
}
impl Interval {
    fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        if text.len() < 5 || !text.is_ascii() {
            return Err(format!("invalid interval: {text}"));
        }
        let (first, last) = (text.as_bytes()[0], text.as_bytes()[text.len() - 1]);
        if !matches!(first, b'[' | b'(') || !matches!(last, b']' | b')') {
            return Err(format!("invalid interval: {text}"));
        }
        let (low, high) = text[1..text.len() - 1]
            .split_once(',')
            .ok_or_else(|| format!("invalid interval: {text}"))?;
        let bound = |s: &str| -> Result<f64> {
            match s.trim() {
                "-inf" => Ok(f64::NEG_INFINITY),
                "inf" => Ok(f64::INFINITY),
                number => number
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| format!("invalid interval bound: {s}")),
            }
        };
        let interval = Self {
            min: bound(low)?,
            max: bound(high)?,
            include_min: first == b'[',
            include_max: last == b']',
        };
        if interval.min > interval.max
            || interval.min == f64::INFINITY
            || interval.max == f64::NEG_INFINITY
            || (interval.min == interval.max && !(interval.include_min && interval.include_max))
            || (!interval.min.is_finite() && interval.include_min)
            || (!interval.max.is_finite() && interval.include_max)
        {
            return Err(format!("empty or invalid interval: {text}"));
        }
        Ok(interval)
    }
    fn contains(&self, v: f64) -> bool {
        v.is_finite()
            && (v > self.min || (self.include_min && v == self.min))
            && (v < self.max || (self.include_max && v == self.max))
    }
    fn overlaps(&self, other: &Self) -> bool {
        !(self.max < other.min
            || other.max < self.min
            || (self.max == other.min && !(self.include_max && other.include_min))
            || (other.max == self.min && !(other.include_max && self.include_min)))
    }
}

impl Model {
    pub fn run(&self, inputs: HashMap<String, Field>, grid: Grid) -> Result<Vec<f64>> {
        self.run_with_layers(inputs, grid).map(|(output, _)| output)
    }
    pub fn run_with_layers(
        &self,
        inputs: HashMap<String, Field>,
        grid: Grid,
    ) -> Result<(Vec<f64>, BTreeMap<String, NodeOutputs>)> {
        self.compile()?.run_with_layers(inputs, grid)
    }
}
impl Graph<'_> {
    pub fn run_with_layers(
        &self,
        mut inputs: HashMap<String, Field>,
        grid: Grid,
    ) -> Result<(Vec<f64>, BTreeMap<String, NodeOutputs>)> {
        let model = self.model;
        if grid.nrows == 0
            || grid.ncols == 0
            || !grid.pixel_width.is_finite()
            || grid.pixel_width <= 0.0
            || grid.nrows.checked_mul(grid.ncols).is_none()
        {
            return Err("invalid model grid".into());
        }
        if let Some(spec) = &model.grid {
            let (rows, cols) = spec.dimensions()?;
            if (rows, cols) != (grid.nrows, grid.ncols)
                || (spec.cellsize - grid.pixel_width).abs() > spec.cellsize * 1e-9
            {
                return Err("model arrays do not match the explicit grid".into());
            }
        } else if !model.inputs.values().any(Input::is_raster) {
            return Err("model requires at least one raster input or an explicit grid".into());
        }
        for (name, spec) in &model.inputs {
            let field = inputs
                .get(name)
                .ok_or_else(|| format!("missing input: {name}"))?;
            match (spec, field) {
                (Input::RaycasterPointSource {}, Field::Points(values))
                    if values.len() % 3 == 0 =>
                {
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
                    | Input::VectorDistance {}
                    | Input::VectorOccluder {}
                    | Input::VectorResistance {},
                    Field::Raster(values),
                ) if values.len() == grid.nrows * grid.ncols => {}
                _ => return Err(format!("wrong input type or dimensions: {name}")),
            }
        }
        if inputs.keys().any(|name| !model.inputs.contains_key(name)) {
            return Err("undeclared model input supplied".into());
        }
        for field in inputs.values_mut() {
            if let Field::Points(points) = field {
                let retained = points
                    .chunks_exact(3)
                    .filter(|point| {
                        point[0] >= 0.0
                            && point[1] >= 0.0
                            && point[0] < grid.ncols as f64
                            && point[1] < grid.nrows as f64
                    })
                    .flatten()
                    .copied()
                    .collect();
                *points = Arc::new(retained);
            }
        }
        let mut evaluator = Evaluator {
            graph: self,
            grid,
            inputs,
            outputs: HashMap::new(),
        };
        let mut exports = BTreeMap::new();
        for name in model.nodes.keys() {
            exports.insert(name.clone(), evaluator.node(name)?);
        }
        let output = evaluator.resolve(&model.output)?.values()?.to_vec();
        Ok((output, exports))
    }
}
struct Evaluator<'g, 'm> {
    graph: &'g Graph<'m>,
    grid: Grid,
    inputs: HashMap<String, Field>,
    outputs: HashMap<String, NodeOutputs>,
}
impl Evaluator<'_, '_> {
    fn resolve(&mut self, reference: &Reference) -> Result<Field> {
        match reference {
            Reference::Input { from_input } => self
                .inputs
                .get(from_input)
                .cloned()
                .ok_or_else(|| format!("unknown input: {from_input}")),
            Reference::Node { from_node, output } => self
                .node(from_node)?
                .get(output.as_deref().unwrap_or("data"))
                .cloned()
                .ok_or_else(|| format!("unknown output on node {from_node}")),
        }
    }
    fn node(&mut self, name: &str) -> Result<NodeOutputs> {
        if let Some(outputs) = self.outputs.get(name) {
            return Ok(outputs.clone());
        }
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
            .apply(&node.operation, &inputs)
            .map_err(|error| format!("node {name}: {error}"))?;
        self.outputs.insert(name.into(), outputs.clone());
        Ok(outputs)
    }
    fn apply(&self, op: &Operation, inputs: &BTreeMap<String, Vec<Field>>) -> Result<NodeOutputs> {
        use Operation::*;
        let one = |name: &str| -> Result<&Field> {
            inputs
                .get(name)
                .and_then(|values| values.first())
                .ok_or_else(|| format!("missing input port {name}"))
        };
        let raster = || -> Result<Vec<f64>> { Ok(one("data")?.values()?.to_vec()) };
        let field: Result<Field> = match op {
            Identity => Ok(one("data")?.clone()),
            Round => Ok(Field::raster(
                raster()?.into_iter().map(f64::round).collect(),
            )),
            DiscreteMap {
                intervals,
                unmatched,
            } => {
                let rules = mapping_rules(intervals)?;
                let mut values = raster()?;
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
                Ok(Field::raster(values))
            }
            Distance {
                units: DistanceUnits::Metres,
                when_empty,
            } => {
                let empty = match when_empty {
                    EmptyDistance::Number(v) if v.is_finite() && *v >= 0.0 => {
                        *v / self.grid.pixel_width
                    }
                    EmptyDistance::Named(Infinity::Infinity) => f64::INFINITY,
                    _ => return Err("empty distance must be nonnegative".into()),
                };
                let field = one("data")?;
                let mut distances =
                    distance_generation(field.values()?, self.grid.nrows, self.grid.ncols, empty);
                for d in &mut distances {
                    *d *= self.grid.pixel_width;
                }
                Ok(Field::raster(distances))
            }
            PowerResponse {
                input_range: [lo, hi],
                base_range: [start, end],
                exponent,
                scale,
                offset,
                above,
            } => {
                let mut values = raster()?;
                for (i, value) in values.iter_mut().enumerate() {
                    if value.is_nan() {
                        continue;
                    }
                    if *value > *hi {
                        *value = above.ok_or_else(|| {
                            format!("value {value} above input range at cell {i}; specify above")
                        })?;
                    } else if !value.is_finite() || *value < *lo {
                        return Err(format!("value {value} below input range at cell {i}"));
                    } else {
                        let base = start + (end - start) * ((*value - lo) / (hi - lo));
                        *value = power_response(base, *exponent, *scale) + offset;
                        if !value.is_finite() {
                            return Err(format!("nonfinite power response at cell {i}"));
                        }
                    }
                }
                Ok(Field::raster(values))
            }
            Override { interval, value } => {
                let interval = Interval::parse(interval)?;
                let mask = one("where")?;
                let mut values = raster()?;
                for (v, &m) in values.iter_mut().zip(mask.values()?) {
                    if interval.contains(m) {
                        *v = *value;
                    }
                }
                Ok(Field::raster(values))
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
                Ok(Field::raster(values))
            }
            Normalise {
                method: Normalisation::PositiveMaximum,
                when_zero,
            } => {
                let mut values = raster()?;
                if values.iter().any(|v| v.is_infinite() || *v < 0.0) {
                    return Err("positive maximum normalisation requires nonnegative values".into());
                }
                let max = values
                    .iter()
                    .copied()
                    .filter(|v| v.is_finite())
                    .fold(0.0, f64::max);
                for v in &mut values {
                    if v.is_finite() {
                        *v = if max > 0.0 { *v / max } else { *when_zero };
                    }
                }
                Ok(Field::raster(values))
            }
            SurfaceHeights {} => {
                let topography = one("topography")?;
                let surface = one("surface")?;
                let occluders = one("occluders")?;
                let output = calc_surfs(
                    topography.values()?,
                    surface.values()?,
                    occluders.values()?,
                    self.grid.nrows,
                    self.grid.ncols,
                );
                return Ok(BTreeMap::from([
                    ("translucent".into(), Field::raster(output.soft_surf)),
                    ("opaque".into(), Field::raster(output.hard_surf)),
                ]));
            }
            Maximum {} | Sum {} => {
                let fields = &inputs["data"];
                let (first, rest) = fields
                    .split_first()
                    .ok_or("aggregation requires at least one input")?;
                let mut values = first.values()?.to_vec();
                for field in rest {
                    for (v, &other) in values.iter_mut().zip(field.values()?) {
                        *v = if !v.is_finite() || !other.is_finite() {
                            f64::NAN
                        } else if matches!(op, Sum {}) {
                            *v + other
                        } else {
                            v.max(other)
                        };
                    }
                }
                for v in &mut values {
                    if !v.is_finite() {
                        *v = f64::NAN;
                    }
                }
                Ok(Field::raster(values))
            }
            Irradiance {
                cutoff_metres,
                sensor_height_metres,
                absorption,
            } => {
                let Field::Points(points) = one("point_sources")? else {
                    return Err("irradiance requires raycaster_point_source data".into());
                };
                let topography = one("topography")?;
                let translucent = one("translucent_surface")?;
                let opaque = one("opaque_surface")?;
                Ok(Field::raster(irradiance_run(
                    &points,
                    translucent.values()?,
                    opaque.values()?,
                    topography.values()?,
                    self.grid.nrows,
                    self.grid.ncols,
                    self.grid.pixel_width,
                    *cutoff_metres,
                    *sensor_height_metres,
                    *absorption,
                )))
            }
        };
        Ok(BTreeMap::from([("data".into(), field?)]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resistance::pipeline::{run_resistance_pipeline, ResistanceParams};
    use serde_json::{json, Value};

    fn example() -> Model {
        serde_json::from_str(include_str!("../../example/horseshoe-bat-model.json")).unwrap()
    }
    fn grid() -> Grid {
        Grid {
            nrows: 1,
            ncols: 6,
            pixel_width: 2.0,
        }
    }
    fn small_model(operations: Value) -> Model {
        serde_json::from_value(json!({
          "name": "test",
          "inputs": {
            "data": {
              "type": "raster_ranked"
            }
          },
          "nodes": {
            "result": {
              "input": "inputs.data",
              "operations": operations
            }
          },
          "output": "nodes.result"
        }))
        .unwrap()
    }
    fn run_small(model: &Model, values: Vec<f64>) -> Result<Vec<f64>> {
        model.run(
            HashMap::from([("data".into(), Field::raster(values))]),
            grid(),
        )
    }
    fn assert_same(actual: &[f64], expected: &[f64]) {
        assert_eq!(actual.len(), expected.len());
        for (i, (&a, &b)) in actual.iter().zip(expected).enumerate() {
            assert!(
                a.is_nan() && b.is_nan() || (a - b).abs() <= 1e-12 * b.abs().max(1.0),
                "cell {i}: {a} != {b}"
            );
        }
    }

    #[test]
    fn raycaster_outside_points_do_not_contribute_or_error() {
        let model: Model = serde_json::from_value(serde_json::json!({
            "name": "lights", "inputs": {
                "terrain": {"type": "raster_topography"},
                "buildings": {"type": "vector_occluder"},
                "lights": {"type": "raycaster_point_source"}
            },
            "nodes": {"light": {
                "op": "irradiance", "inputs": {
                    "point_sources": "inputs.lights",
                    "topography": "inputs.terrain",
                    "opaque_surface": "inputs.buildings",
                    "translucent_surface": "inputs.buildings"
                },
                "parameters": {"cutoff_metres": 10, "sensor_height_metres": 0, "absorption": 0.5}
            }}, "output": "nodes.light"
        }))
        .unwrap();
        let mut fields = HashMap::from([
            ("terrain".into(), Field::raster(vec![0.0; 4])),
            ("buildings".into(), Field::raster(vec![0.0; 4])),
            (
                "lights".into(),
                Field::Points(Arc::new(vec![
                    -1.0, 0.0, 3.0, 2.0, 0.0, 3.0, 0.0, -1.0, 3.0, 0.0, 2.0, 3.0,
                ])),
            ),
        ]);
        let grid = Grid {
            nrows: 2,
            ncols: 2,
            pixel_width: 1.0,
        };
        assert_eq!(model.run(fields.clone(), grid).unwrap(), vec![0.0; 4]);
        fields.insert(
            "lights".into(),
            Field::Points(Arc::new(vec![-1.0, 0.0, -3.0])),
        );
        assert!(model
            .run(fields, grid)
            .unwrap_err()
            .contains("invalid point coordinates or height"));
    }

    #[test]
    fn interval_mapping_respects_endpoints_unmatched_and_missing() {
        let model = small_model(json!([{
          "op": "discrete_map",
          "parameters": {
            "intervals": {
              "(1,3)": 4,
              "(3,6)": 1,
              "[6,inf)": 2
            },
            "unmatched": 0
          }
        }]));
        assert_same(
            &run_small(&model, vec![1.0, 2.0, 3.0, 4.0, 6.0, f64::NAN]).unwrap(),
            &[0.0, 4.0, 0.0, 1.0, 2.0, f64::NAN],
        );
        for (text, expected) in [
            ("[1,3)", vec![1.0, 2.0]),
            ("(3,5]", vec![4.0, 5.0]),
            ("[4,4]", vec![4.0]),
        ] {
            let interval = Interval::parse(text).unwrap();
            let actual: Vec<f64> = (0..7)
                .map(|v| v as f64)
                .filter(|v| interval.contains(*v))
                .collect();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn invalid_intervals_and_overlaps_are_errors() {
        for text in [
            "[3,1]",
            "(3,3)",
            "[inf,inf]",
            "[-inf,1]",
            "[1,2",
            "[NaN,2]",
            "[1,2,3]",
            "é1,3)",
        ] {
            assert!(Interval::parse(text).is_err(), "{text}");
        }
        for intervals in [
            json!({
              "[1,3]": 1,
              "[3,5]": 2
            }),
            json!({
              "(0,10)": 1,
              "[2,3]": 2
            }),
        ] {
            let model = small_model(json!([{
              "op": "discrete_map",
              "parameters": {
                "intervals": intervals,
                "unmatched": 0
              }
            }]));
            let error = run_small(&model, vec![0.0; 6]).unwrap_err();
            assert!(
                error.contains("node result: overlapping interval"),
                "{error}"
            );
        }
    }

    #[test]
    fn unmatched_error_and_nodata_are_distinct() {
        let op = json!({
          "op": "discrete_map",
          "parameters": {
            "intervals": {
              "[1,1]": 2
            },
            "unmatched": "error"
          }
        });
        let model = small_model(json!([op]));
        assert!(run_small(&model, vec![3.0; 6])
            .unwrap_err()
            .contains("unmatched value 3 at cell 0"));
        let model = small_model(json!([{
          "op": "discrete_map",
          "parameters": {
            "intervals": {},
            "unmatched": "nodata"
          }
        }]));
        assert!(run_small(&model, vec![3.0; 6])
            .unwrap()
            .iter()
            .all(|v| v.is_nan()));
    }

    #[test]
    fn distance_response_preserves_boundary_empty_and_missing_rules() {
        let model = small_model(json!([{
          "op": "distance",
          "parameters": {
            "units": "metres",
            "when_empty": "infinity"
          }
        }, {
          "op": "power_response",
          "parameters": {
            "input_range": [0, 4],
            "base_range": [1, 0.5],
            "exponent": 2,
            "scale": 16,
            "offset": 0,
            "above": 0
          }
        }]));
        assert_same(
            &run_small(&model, vec![1.0, 0.0, 0.0, 0.0, f64::NAN, 0.0]).unwrap(),
            &[16.0, 9.0, 4.0, 0.0, f64::NAN, 0.0],
        );
        assert_same(
            &run_small(&model, vec![0.0, 0.0, 0.0, 0.0, f64::NAN, 0.0]).unwrap(),
            &[0.0, 0.0, 0.0, 0.0, f64::NAN, 0.0],
        );
    }

    #[test]
    fn invalid_responses_references_cycles_and_types_are_errors() {
        for response in [
            json!({
              "op": "power_response",
              "parameters": {
                "input_range": [0, 0],
                "base_range": [0, 1],
                "exponent": 1,
                "scale": 1,
                "offset": 0
              }
            }),
            json!({
              "op": "power_response",
              "parameters": {
                "input_range": [0, 1],
                "base_range": [0, 1],
                "exponent": 0,
                "scale": 1,
                "offset": 0
              }
            }),
            json!({
              "op": "power_response",
              "parameters": {
                "input_range": [0, 1],
                "base_range": [0, 1],
                "exponent": 1,
                "scale": 1,
                "offset": 0
              }
            }),
            json!({
              "op": "power_response",
              "parameters": {
                "input_range": [0, 10],
                "base_range": [-1, -1],
                "exponent": 0.5,
                "scale": 1,
                "offset": 0
              }
            }),
        ] {
            assert!(run_small(&small_model(json!([response])), vec![2.0; 6]).is_err());
        }
        let mut model = small_model(json!([]));
        model.nodes.insert(
            "result".into(),
            NodeSpec::Chain {
                input: Reference::Input {
                    from_input: "missing".into(),
                },
                operations: vec![],
            },
        );
        assert!(run_small(&model, vec![0.0; 6])
            .unwrap_err()
            .contains("unknown input: missing"));
        model.nodes.insert(
            "result".into(),
            NodeSpec::Chain {
                input: Reference::node("result".into()),
                operations: vec![],
            },
        );
        assert!(run_small(&model, vec![0.0; 6])
            .unwrap_err()
            .contains("cyclic node reference"));
        model.nodes.insert(
            "result".into(),
            NodeSpec::Node(Node {
                operation: Operation::Sum {},
                inputs: BTreeMap::from([("data".into(), Connection::Many(vec![]))]),
            }),
        );
        assert!(run_small(&model, vec![0.0; 6])
            .unwrap_err()
            .contains("at least one reference"));
        model.nodes.insert(
            "result".into(),
            NodeSpec::Node(Node {
                operation: Operation::Round,
                inputs: BTreeMap::new(),
            }),
        );
        assert!(run_small(&model, vec![0.0; 6])
            .unwrap_err()
            .contains("missing input port data"));
        model = small_model(json!([]));
        assert!(model
            .run(
                HashMap::from([("data".into(), Field::raster(vec![0.0; 5]))]),
                grid()
            )
            .is_err());
        model
            .inputs
            .insert("data".into(), Input::RaycasterPointSource {});
        assert!(model
            .run(
                HashMap::from([("data".into(), Field::Points(Arc::new(vec![])))]),
                grid()
            )
            .is_err());
    }

    #[test]
    fn model_schema_rejects_unknown_operations_fields_and_policies() {
        let original: Value =
            serde_json::from_str(include_str!("../../example/horseshoe-bat-model.json")).unwrap();
        for (input, alias) in [("buildings", "value_field"), ("lamps", "height_field")] {
            let mut value = original.clone();
            value["inputs"][input][alias] = json!("value");
            assert!(serde_json::from_value::<Model>(value).is_err());
        }
        for (input, key, setting) in [
            ("roads", "required_fields", json!(["presence"])),
            ("buildings", "background", json!(0)),
        ] {
            let mut value = original.clone();
            value["inputs"][input][key] = setting;
            assert!(serde_json::from_value::<Model>(value).is_err());
        }
        for (pointer, value) in [
            ("/nodes/road_res/operations/0/op", json!("script")),
            (
                "/nodes/tree_res/operations/1/parameters/units",
                json!("feet"),
            ),
            (
                "/nodes/tree_res/operations/1/parameters/when_empty",
                json!("zero"),
            ),
            ("/nodes/road_res/operations/0/typo", json!(1)),
        ] {
            let mut value_copy = original.clone();
            if let Some(slot) = value_copy.pointer_mut(pointer) {
                *slot = value;
            } else {
                value_copy["nodes"]["road_res"]["operations"][0]["typo"] = value;
            }
            assert!(
                serde_json::from_value::<Model>(value_copy).is_err(),
                "{pointer}"
            );
        }
    }

    #[test]
    fn horseshoe_spec_matches_every_existing_resistance_layer() {
        let (m, n) = (13, 17);
        let total = m * n;
        for pixel_width in [1.0, 2.0, 5.0] {
            for scenario in 0..4 {
                let mut dtm = vec![100.0; total];
                let heights = [0.0, 1.0, 2.0, 3.0, 4.0, 6.0, 8.0];
                let mut dsm: Vec<f64> = (0..total)
                    .map(|i| 100.0 + heights[i % heights.len()])
                    .collect();
                let mut buildings = vec![0.0; total];
                let mut lcm: Vec<f64> = (0..total).map(|i| (i % 23) as f64).collect();
                let mut roads = vec![0.0; total];
                let mut rivers = vec![0.0; total];
                let mut generic = vec![3.0; total];
                let lamps = if scenario == 0 {
                    vec![]
                } else {
                    vec![4.0, 5.0, 8.0, 12.0, 8.0, 5.0]
                };
                if scenario > 0 {
                    roads[0] = 1.0;
                    roads[total - 1] = 2.0;
                    rivers[n + 2] = 1.0;
                }
                if scenario == 2 {
                    roads.fill(1.0);
                    rivers.fill(1.0);
                }
                if scenario == 3 {
                    dsm.fill(102.0);
                }
                buildings[10] = 5.0;
                buildings[11] = 4.0;
                buildings[12] = 2.0;
                dsm[10] = f64::NAN;
                dtm[11] = f64::NAN;
                lcm[12] = f64::NAN;
                dtm[13] = f64::NAN;
                dsm[14] = f64::NAN;
                roads[15] = f64::NAN;
                rivers[16] = f64::NAN;
                generic[17] = f64::NAN;
                let params = ResistanceParams {
                    road_buffer: 200.0,
                    road_resmax: 10.0,
                    road_xmax: 5.0,
                    river_buffer: 10.0,
                    river_resmax: 2000.0,
                    river_xmax: 4.0,
                    landscape_rankmax: 8.0,
                    landscape_resmax: 100.0,
                    landscape_xmax: 5.0,
                    linear_buffer: 10.0,
                    linear_rankmax: 4.0,
                    linear_resmax: 22000.0,
                    linear_xmax: 3.0,
                    lamp_resmax: 1e8,
                    lamp_xmax: 1.0,
                    lamp_ext: 100.0,
                    pixw: pixel_width,
                    nrows: m,
                    ncols: n,
                };
                let expected = run_resistance_pipeline(
                    &roads,
                    &rivers,
                    &buildings,
                    Some(&lcm),
                    &dtm,
                    &dsm,
                    &generic,
                    &lamps,
                    &params,
                    None,
                    None,
                );
                let mut fields = HashMap::new();
                for (name, values) in [
                    ("dtm", dtm),
                    ("dsm", dsm),
                    ("landcover", lcm),
                    ("buildings", buildings),
                    ("roads", roads),
                    ("rivers", rivers),
                    ("generic", generic),
                ] {
                    let mut values = values;
                    if name == "roads" || name == "rivers" {
                        values = distance_generation(&values, m, n, f64::INFINITY);
                        for value in &mut values {
                            *value *= pixel_width;
                        }
                    }
                    let field = Field::raster(values);
                    fields.insert(name.into(), field);
                }
                fields.insert("lamps".into(), Field::Points(Arc::new(lamps)));
                let mut model = example();
                for (name, expected) in [
                    ("road_res", expected.road_res),
                    ("river_res", expected.river_res),
                    ("landscape_res", expected.landscape_res),
                    ("linear_res", expected.linear_res),
                    ("lamp_res", expected.lamp_res),
                    ("total_res", expected.total_res),
                    ("surfaces.translucent", expected.soft_surf),
                    ("surfaces.opaque", expected.hard_surf),
                ] {
                    model.output = if let Some((node, port)) = name.split_once('.') {
                        Reference::Node {
                            from_node: node.into(),
                            output: Some(port.into()),
                        }
                    } else {
                        Reference::node(name.into())
                    };
                    let actual = model
                        .run(
                            fields.clone(),
                            Grid {
                                nrows: m,
                                ncols: n,
                                pixel_width,
                            },
                        )
                        .unwrap();
                    assert_same(&actual, &expected);
                }
            }
        }
    }
    #[test]
    fn canonical_nodes_and_compact_chains_execute_the_same_graph() {
        let chain = small_model(json!([
            {"op":"discrete_map","parameters":{"intervals":{"[0,2]":2,"[3,4]":4},"unmatched":"error"}},
            {"op":"power_response","parameters":{"input_range":[0,4],"base_range":[0,1],"exponent":2,"scale":100,"offset":0}}
        ]));
        let explicit:Model=serde_json::from_value(json!({
            "name":"explicit","inputs":{"data":{"type":"raster_ranked"}},
            "nodes":{
                "a_response":{"op":"power_response","inputs":"nodes.z_map",
                    "parameters":{"input_range":[0,4],"base_range":[0,1],"exponent":2,"scale":100,"offset":0}},
                "z_map":{"op":"discrete_map","inputs":"inputs.data",
                    "parameters":{"intervals":{"[0,2]":2,"[3,4]":4},"unmatched":"error"}}
            },"output":"nodes.a_response"
        })).unwrap();
        let values = vec![0.0, 1.0, 2.0, 3.0, 4.0, f64::NAN];
        let expected = [25.0, 25.0, 25.0, 100.0, 100.0, f64::NAN];
        assert_same(&run_small(&chain, values.clone()).unwrap(), &expected);
        assert_same(&run_small(&explicit, values).unwrap(), &expected);
        let graph = chain.compile().unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.nodes["result"].inputs.len(), 1);
    }

    #[test]
    fn graph_checks_named_ports_types_units_and_cycles_without_loading_data() {
        let mut model = example();
        model.output = Reference::node("surfaces".into());
        assert!(model
            .compile()
            .err()
            .unwrap()
            .contains("select a declared output port"));
        model.output = Reference::Node {
            from_node: "surfaces".into(),
            output: Some("opaque".into()),
        };
        assert!(model.compile().is_ok());
        model.output = Reference::Node {
            from_node: "surfaces".into(),
            output: Some("missing".into()),
        };
        assert!(model
            .compile()
            .err()
            .unwrap()
            .contains("has no output missing"));
        let mut model = example();
        let NodeSpec::Node(node) = model.nodes.get_mut("irradiance").unwrap() else {
            panic!()
        };
        node.inputs.insert(
            "point_sources".into(),
            Connection::One(Reference::Input {
                from_input: "dtm".into(),
            }),
        );
        assert!(model
            .compile()
            .err()
            .unwrap()
            .contains("point_sources must be raycaster_point_source"));
        let mut model = example();
        let NodeSpec::Node(node) = model.nodes.get_mut("total_res").unwrap() else {
            panic!()
        };
        node.inputs.insert(
            "data".into(),
            Connection::Many(vec![
                Reference::Input {
                    from_input: "roads".into(),
                },
                Reference::Input {
                    from_input: "generic".into(),
                },
            ]),
        );
        assert!(model.compile().err().unwrap().contains("same units"));
        let mut model = small_model(json!([]));
        model.nodes.insert(
            "result".into(),
            NodeSpec::Node(Node {
                operation: Operation::Round,
                inputs: BTreeMap::from([(
                    "data".into(),
                    Connection::Many(vec![Reference::Input {
                        from_input: "data".into(),
                    }]),
                )]),
            }),
        );
        assert!(model
            .compile()
            .err()
            .unwrap()
            .contains("expects one reference"));
        model.nodes.insert(
            "result".into(),
            NodeSpec::Node(Node {
                operation: Operation::Round,
                inputs: BTreeMap::from([
                    (
                        "data".into(),
                        Connection::One(Reference::Input {
                            from_input: "data".into(),
                        }),
                    ),
                    (
                        "typo".into(),
                        Connection::One(Reference::Input {
                            from_input: "data".into(),
                        }),
                    ),
                ]),
            }),
        );
        assert!(model
            .compile()
            .err()
            .unwrap()
            .contains("unknown input port typo"));
        model.nodes.insert(
            "result".into(),
            NodeSpec::Chain {
                input: Reference::node("other".into()),
                operations: vec![],
            },
        );
        model.nodes.insert(
            "other".into(),
            NodeSpec::Chain {
                input: Reference::node("result".into()),
                operations: vec![],
            },
        );
        assert!(model
            .compile()
            .err()
            .unwrap()
            .contains("cyclic node reference"));
    }

    #[test]
    fn references_and_parameters_are_not_ambiguous_strings_or_flat_options() {
        for definition in [
            json!({"op":"round", "inputs":{"data":"inputs.data"}}),
            json!({"op":"sum", "inputs":{"data":["inputs.data"]}}),
        ] {
            assert!(serde_json::from_value::<Node>(definition).is_err());
        }
        for invalid in [
            "data",
            "inputs.",
            "inputs.roads.extra",
            "nodes.",
            "nodes.surfaces.",
            "nodes.surfaces.opaque.extra",
            "other.roads",
        ] {
            assert!(
                serde_json::from_value::<Reference>(json!(invalid)).is_err(),
                "{invalid}"
            );
        }
        for invalid in [
            json!({"from_input": "roads"}),
            json!({"from_node": "road_res"}),
        ] {
            assert!(serde_json::from_value::<Reference>(invalid).is_err());
        }
        let reference: Reference = serde_json::from_value(json!("nodes.surfaces.opaque")).unwrap();
        assert!(reference.selects_node("surfaces", "opaque"));
        let reference: Reference = serde_json::from_value(json!("nodes.road_res")).unwrap();
        assert!(reference.selects_node("road_res", "data"));
        assert!(
            matches!(serde_json::from_value::<Reference>(json!("inputs.roads")).unwrap(), Reference::Input { from_input } if from_input == "roads")
        );
        assert!(serde_json::from_value::<Reference>(
            json!({"from_input":"data","from_node":"other"})
        )
        .is_err());
        assert!(serde_json::from_value::<Node>(
            json!({"op":"round","inputs":"inputs.data","typo":0})
        )
        .is_err());
        assert!(
            serde_json::from_value::<Node>(json!({"op":"round","parameters":{"op":"sum"}}))
                .is_err()
        );
        let model = small_model(
            json!([{"op":"power_response","parameters":{"input_range":[0,0],"base_range":[0,1],"exponent":2,"scale":1,"offset":0}}]),
        );
        assert!(model
            .compile()
            .err()
            .unwrap()
            .contains("invalid power response parameters"));
    }
}
