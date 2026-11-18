//! Analytical instrumentation of the connectivity solver's named buffers.

mod sizes;
mod report;
mod recording;

pub use recording::*;
pub use report::*;
pub use sizes::*;

#[cfg(test)]
mod tests;
