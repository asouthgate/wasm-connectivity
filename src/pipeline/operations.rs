use super::{model::schema::Input, Result};
use crate::resistance::operations::interval::{parse_interval_mappings, Interval};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Current {
        #[serde(skip)]
        current_solver_settings: crate::pipeline::CurrentSolverSettings,
    },
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
pub use crate::resistance::operations::Unmatched;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::pipeline) enum Kind {
    Raster,
    Topography,
    Surface,
    Ranked,
    Distance,
    Occluder,
    Height,
    Resistance,
    Irradiance,
    Current,
    Lights,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::pipeline) enum Unit {
    Metres,
    Dimensionless,
    Resistance,
    RelativeIrradiance,
    Current,
    None,
}
#[derive(Debug, Clone, Copy)]
pub(in crate::pipeline) struct Descriptor {
    pub(in crate::pipeline) kind: Kind,
    pub(in crate::pipeline) unit: Unit,
}
impl Descriptor {
    fn new(kind: Kind, unit: Unit) -> Self {
        Self { kind, unit }
    }
    pub(in crate::pipeline) fn is_raster(self) -> bool {
        self.kind != Kind::Lights
    }
}
impl Input {
    pub(in crate::pipeline) fn descriptor(&self) -> Descriptor {
        use Kind::*;
        match self {
            Self::RasterTopography {} => Descriptor::new(Topography, Unit::Metres),
            Self::RasterSurface {} => Descriptor::new(Surface, Unit::Metres),
            Self::RasterRanked {} => Descriptor::new(Ranked, Unit::Dimensionless),
            Self::VectorMask {} => Descriptor::new(Raster, Unit::Dimensionless),
            Self::VectorOccluder {} => Descriptor::new(Occluder, Unit::Metres),
            Self::VectorResistance {} => Descriptor::new(Resistance, Unit::Resistance),
            Self::RaycasterPointSource {} => Descriptor::new(Lights, Unit::None),
        }
    }
}
#[derive(Clone, Copy)]
pub(in crate::pipeline) enum InputTypes {
    Any,
    Raster,
    Topography,
    Surface,
    Occluder,
    Height,
    Lights,
}
impl InputTypes {
    pub(in crate::pipeline) fn accepts(self, descriptor: Descriptor) -> bool {
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
    pub(in crate::pipeline) fn name(self) -> &'static str {
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
pub(in crate::pipeline) struct Port {
    pub(in crate::pipeline) name: &'static str,
    pub(in crate::pipeline) accepts: InputTypes,
    pub(in crate::pipeline) many: bool,
}
impl Operation {
    pub(in crate::pipeline) fn output_names(&self) -> &'static [&'static str] {
        match self {
            Self::SurfaceHeights {} => &["translucent", "opaque"],
            _ => &["data"],
        }
    }

    pub(in crate::pipeline) fn ports(&self) -> &'static [Port] {
        use InputTypes::*;
        match self {
            Self::Current { .. } => &[
                Port {
                    name: "resistance",
                    accepts: Raster,
                    many: false,
                },
                Port {
                    name: "source",
                    accepts: Raster,
                    many: false,
                },
                Port {
                    name: "ground",
                    accepts: Raster,
                    many: false,
                },
            ],
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
    pub(in crate::pipeline) fn output_types(
        &self,
        inputs: &BTreeMap<String, Vec<Descriptor>>,
    ) -> Result<BTreeMap<String, Descriptor>> {
        use Kind::*;
        let data = || inputs["data"][0];
        let descriptor = match self {
            Self::Identity | Self::Round | Self::Override { .. } | Self::MaskNa {} => data(),
            Self::Current { .. } => Descriptor::new(Current, Unit::Current),
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
    pub(in crate::pipeline) fn validate_parameters(&self) -> Result<()> {
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
