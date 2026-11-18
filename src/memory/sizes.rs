use crate::linalg::real::Real;
use sprs::CsMat;

/// Byte width of a `usize`. Native (64-bit) is 8; `wasm32` is 4.
#[inline]
pub fn usize_size() -> u64 {
    std::mem::size_of::<usize>() as u64
}

/// Bytes of a `Vec<f64>` of length `n`.
#[inline]
pub fn vec_f64_bytes(n: usize) -> u64 { vec_scalar_bytes::<f64>(n) }

#[inline]
#[allow(unused_variables)]
pub fn vec_scalar_bytes<F: Real>(n: usize) -> u64 {
    n as u64 * std::mem::size_of::<F>() as u64
}

/// Bytes of a `Vec<i32>` of length `n`.
#[inline]
pub fn vec_i32_bytes(n: usize) -> u64 {
    n as u64 * 4
}

/// Bytes of a `Vec<usize>` of length `n`.
#[inline]
pub fn vec_usize_bytes(n: usize) -> u64 {
    n as u64 * usize_size()
}

/// Bytes of a sparse CSR matrix (`indptr` + `indices` + `data`).
#[inline]
pub fn csmat_bytes<F: Real>(a: &CsMat<F>) -> u64 {
    let u = usize_size();
    (a.rows() as u64 + 1) * u + a.nnz() as u64 * u + a.nnz() as u64 * std::mem::size_of::<F>() as u64
}

/// Bytes of `n` triplets stored as three parallel vectors
/// (`row: Vec<usize>`, `col: Vec<usize>`, `val: Vec<f64>`).
#[inline]
pub fn triplets_bytes(n: usize) -> u64 { triplets_bytes_typed::<f64>(n) }

#[inline]
#[allow(unused_variables)]
pub fn triplets_bytes_typed<F: Real>(n: usize) -> u64 {
    n as u64 * (2 * usize_size() + std::mem::size_of::<F>() as u64)
}

/// Bytes of a dense `n x n` `f64` matrix.
#[inline]
pub fn dense_bytes(n: usize) -> u64 { dense_bytes_typed::<f64>(n) }

#[inline]
#[allow(unused_variables)]
pub fn dense_bytes_typed<F: Real>(n: usize) -> u64 {
    n as u64 * n as u64 * std::mem::size_of::<F>() as u64
}
