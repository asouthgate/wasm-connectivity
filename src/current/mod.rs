pub mod circuit;
pub mod current;
pub mod linalg;
pub mod memory;
pub mod mixed;

pub use current_solver::{CurrentSolver, CurrentSolverSettings};
pub use explicit::{solve_raster_sources_mg, solve_raster_sources_mg_with_options};
pub use ground::GroundMode;
pub use output::{AnnotatedOutput, RasterOutput};
pub use stencil::{
    solve_raster_sources_mg_stencil, solve_raster_sources_mg_stencil_in_place_with_options,
    solve_raster_sources_mg_stencil_with_options,
};

mod current_solver;
mod explicit;
mod ground;
mod output;
mod residual;
mod sources;
mod stencil;

#[cfg(test)]
pub(crate) use ground::apply_dirichlet_ground_lap;

#[cfg(test)]
mod tests;
