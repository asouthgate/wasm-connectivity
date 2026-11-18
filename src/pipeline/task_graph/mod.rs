//! Build the per-sample task graph and execute its calculations and streaming nodes.

pub(super) mod current_solver;
mod operations;
pub(super) mod scheduler;
mod statistics;

#[cfg(test)]
mod tests;
