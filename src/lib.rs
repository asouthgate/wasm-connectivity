//! lmConnect: Low-memory Biological Connectivity Analysis.
//!
//! Raster-based connectivity analysis for native Rust and WebAssembly.
//!
//! Current and voltage maps are computed from resistance, source, and ground
//! rasters using preconditioned conjugate gradients. Solver variants include
//! geometric multigrid, a low-memory stencil implementation, and mixed precision.
//!
//! # Workflow
//!
//! Rasterization, resistance modelling, and current solving are separate steps:
//!
//! 1. [`geospatial::rasterize_values`] converts GeoJSON features into raster
//!    values.
//! 2. [`pipeline`] computes resistance from aligned input rasters
//!    and lamp data.
//! 3. [`current`] computes current and voltage maps from the resulting resistance
//!    raster and the caller's source and ground rasters.
//!
//! Callers with an existing resistance raster can go directly to the solver.
//! Ground behaviour is selected through [`current::GroundMode`]. See
//! [`raster::fill_nodata`] for the solver's treatment of invalid resistance cells.
//!
//! # Rust usage
//!
//! Use the module APIs for Rust inputs and structured results. For example:
//!
//! ```
//! use lm_connect::current::{solve_raster_sources_mg_stencil, GroundMode};
//!
//! let (nrows, ncols) = (32, 32);
//! let resistance = vec![1.0_f64; nrows * ncols];
//! let mut source = vec![0.0; nrows * ncols];
//! let mut ground = vec![0.0; nrows * ncols];
//! source[0] = 1.0;
//! ground[nrows * ncols - 1] = 1.0;
//!
//! let result = solve_raster_sources_mg_stencil(
//!     &resistance, nrows, ncols, -9999.0, &source, &ground,
//!     100_000, 1e-4, true, GroundMode::Shunt,
//! );
//! assert!(result.error.is_none(), "{:?}", result.error);
//! assert_eq!(result.output.current_map.len(), nrows * ncols);
//! assert_eq!(result.output.voltages.len(), nrows * ncols);
//! ```
//!
//! # JavaScript and WebAssembly
//!
//! The functions at the crate root provide the `wasm-bindgen` interface.
//! Solver exports return JSON-encoded results; rasterization exports return
//! numeric arrays. Resistance exports return [`BrowserResistanceOutput`], whose
//! rasters are consumed as `f32` arrays through
//! [`BrowserResistanceOutput::release_and_convert_f64_raster_to_f32`].
//!
//! The JavaScript wrappers in `lib/` manage worker calls and Wasm initialization
//! for rasterization, resistance computation, and current solving. See
//! `lib/README.md` for integration examples and `lib/index.d.ts` for their types.
//! Direct JavaScript calls such as raster resizing require `await load()` first.
//!
//! # Additional functionality and features
//!
//! [`roost`] provides acoustic-detector-based roost estimation. The `bin` Cargo
//! feature enables native command-line tools and raster file I/O dependencies;
//! `instrumentation-profile` enables solver memory instrumentation. The
//! `developer` feature enables the `lmc-profile` executable and its dependencies.

pub mod current;
pub mod geospatial;
pub mod pipeline;
pub mod raster;
pub mod real;
pub mod resistance;
pub mod roost;

pub use current as solve;
pub use current::{circuit, linalg, memory};

pub use circuit::build_circuit_model;

mod browser;
pub use browser::*;

pub const NODATA_SENTINEL: f64 = -9999.0;
pub const DEFAULT_MAX_ITER: usize = 100_000;
pub const DEFAULT_TOL: f64 = <f64 as real::Real>::DEFAULT_TOLERANCE;
