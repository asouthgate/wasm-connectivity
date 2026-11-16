use super::Result;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub struct Grid {
    pub nrows: usize,
    pub ncols: usize,
    pub pixel_width: f64,
}

impl Grid {
    pub(crate) fn contains_point(&self, point: &[f64]) -> bool {
        point[0] >= 0.0
            && point[1] >= 0.0
            && point[0] < self.ncols as f64
            && point[1] < self.nrows as f64
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct JsonPipeline {
    pub name: String,
    pub grid: Option<GridSpec>,
    pub inputs: BTreeMap<String, Input>,
    pub nodes: BTreeMap<String, NodeSpec>,
    pub output: Reference,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridSpec {
    // Required in JSON, but only file loading uses the coordinate reference system.
    #[cfg_attr(
        not(all(feature = "bin", not(target_arch = "wasm32"))),
        allow(dead_code)
    )]
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
    VectorMask {},
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
    pub(super) fn node(name: String) -> Self {
        Self::Node {
            from_node: name,
            output: None,
        }
    }
    #[cfg(any(test, all(feature = "bin", not(target_arch = "wasm32"))))]
    pub fn selects_node(&self, name: &str, port: &str) -> bool {
        matches!(self, Self::Node { from_node, output } if from_node == name && output.as_deref().unwrap_or("data") == port)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum InputEdges {
    One(Reference),
    Many(Vec<Reference>),
}
impl InputEdges {
    pub(super) fn references(&self) -> &[Reference] {
        match self {
            Self::One(value) => std::slice::from_ref(value),
            Self::Many(values) => values,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub operation: Operation,
    pub inputs: BTreeMap<String, InputEdges>,
    pub sampling: Option<super::sampling::SampledParameters>,
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
        let mut parameters = serde_json::Value::Object(definition.parameters);
        let sampling = super::sampling::SampledParameters::extract(&mut parameters)
            .map_err(serde::de::Error::custom)?;
        let operation: Operation =
            serde_json::from_value(parameters).map_err(serde::de::Error::custom)?;
        let inputs = match definition.inputs {
            None => BTreeMap::new(),
            Some(value) if operation.ports().len() == 1 && operation.ports()[0].name == "data" => {
                let connection: InputEdges =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                BTreeMap::from([("data".into(), connection)])
            }
            Some(value) => serde_json::from_value(value).map_err(serde::de::Error::custom)?,
        };
        Ok(Self {
            operation,
            inputs,
            sampling,
        })
    }
}

/// Parsed JSON definition: either one operation with explicit inputs (`Node`),
/// or an `input` and `operations` array (`Chain`). Loading expands the array
/// into connected operation nodes.
#[derive(Debug, Clone)]
pub(super) enum NodeSpec {
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
