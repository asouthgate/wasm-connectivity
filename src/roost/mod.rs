//! Bat roost location estimation: core calculation library.
//!
//! Pure, dependency-free implementation of the error-surface method from
//! "A simple and fast method for estimating bat roost locations" (Henley et al.).
//!
//! This module contains only the mathematics: the exponential integral and the
//! error-surface search. Rendering (colormap, colorbar, contours, annotations)
//! lives in the shared frontend plotter.

pub mod exp1;
pub mod io;
pub mod surface;

pub use surface::{compute_error_surface, SurfaceResult};
