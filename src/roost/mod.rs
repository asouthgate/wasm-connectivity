//! Bat roost location estimation: core calculation library.
//!
//! Pure, dependency-free implementation of the error-surface method from
//! "A simple and fast method for estimating bat roost locations" (Henley et al.).
//!
//! This module contains only the mathematics: the exponential integral, the
//! error-surface search, and the parula colormap used for rendering.

pub mod exp1;
pub mod parula;
pub mod surface;

pub use surface::{compute_error_surface, SurfaceResult};
