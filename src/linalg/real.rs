use num_traits::{Float, FromPrimitive, NumAssign};
use serde::Serialize;
use std::{
    fmt::{Debug, Display, LowerExp},
    iter::Sum,
    str::FromStr,
};

mod sealed {
    pub trait Sealed {}
    impl Sealed for f32 {}
    impl Sealed for f64 {}
}

/// Scalar arithmetic shared by the single- and double-precision solvers.
pub trait Real:
    sealed::Sealed
    + Float
    + FromPrimitive
    + NumAssign
    + Sum<Self>
    + Default
    + Debug
    + Display
    + LowerExp
    + Serialize
    + FromStr
    + Send
    + Sync
    + 'static
{
    const DEFAULT_TOLERANCE: Self;
}
impl Real for f32 {
    const DEFAULT_TOLERANCE: Self = 1e-3;
}
impl Real for f64 {
    const DEFAULT_TOLERANCE: Self = 1e-4;
}

#[inline]
pub fn scalar<F: Real>(value: f64) -> F {
    F::from_f64(value).expect("scalar conversion")
}
