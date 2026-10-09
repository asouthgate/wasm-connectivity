//! Abstraction over the fine-grid graph Laplacian `L`.
//!
//! The connectivity solve only needs a handful of operations on `L`: a
//! matrix-vector product (CG), the inverse diagonal (Jacobi preconditioner),
//! row iteration (Gauss-Seidel smoothing and Galerkin coarsening), and a way
//! to report its logical size. [`Operator`] captures exactly that, so the same
//! solver can run over an explicitly materialised CSR matrix or a matrix-free
//! stencil that computes `L` on the fly from the resistance raster.
//!
//! The default (explicit) path uses a `CsMat<f64>` directly, which implements
//! [`Operator`]. [`StencilOperator`] borrows the input resistance raster and
//! reconstructs the 5-point Laplacian row by row from
//! `conductance(i, j) = 2 / (r_i + r_j)`, avoiding a fine CSR matrix and its
//! construction intermediates (conductance grid, cell-to-node map, edge
//! triplets). An uncoarsened small system may use a temporary dense matrix
//! for direct factorization; the retained operator remains a stencil.

use sprs::CsMat;

/// Ground boundary conditions, declared in terms of the full grid (one entry
/// per node, node index == flat cell index).
pub enum GroundSpec {
    /// No physical grounds. The usual numerical regularization is retained;
    /// the solver may separately remove the source mean to balance injection.
    None,
    /// Conductance-to-ground (Neumann): `shunt[i]` is added to `L[i,i]`.
    Neumann(Vec<f64>),
    /// Fixed-voltage (Dirichlet): `ground[i]` pins node `i` at `V = 0`.
    Dirichlet(Vec<bool>),
}

/// A linear operator representing the graph Laplacian `L`.
pub trait Operator {
    /// Number of unknowns (nodes).
    fn n(&self) -> usize;

    /// `y = L x`.
    fn matvec(&self, x: &[f64], y: &mut [f64]);

    /// `1 / |L[i,i]|` per node (used by the Jacobi preconditioner).
    fn diag_inv(&self) -> Vec<f64>;

    /// Invoke `f(col, val)` for every stored entry of row `row` (diagonal and
    /// off-diagonal). For a stencil these are computed on the fly.
    fn for_each_entry(&self, row: usize, f: &mut dyn FnMut(usize, f64));
}

/// The explicit CSR Laplacian implements the operator directly.
impl Operator for CsMat<f64> {
    fn n(&self) -> usize {
        self.rows()
    }

    fn matvec(&self, x: &[f64], y: &mut [f64]) {
        crate::linalg::pcg::mat_vec_mul_slice(self, x, y);
    }

    fn diag_inv(&self) -> Vec<f64> {
        crate::circuit::laplacian::extract_diag_inv(self)
    }

    fn for_each_entry(&self, row: usize, f: &mut dyn FnMut(usize, f64)) {
        if let Some(rv) = self.outer_view(row) {
            for (col, &val) in rv.iter() {
                f(col, val);
            }
        }
    }
}

/// The matrix-free 5-point Laplacian.
///
/// Borrows the input resistance raster and reconstructs `L` on demand: nodata /
/// non-positive / non-finite cells are treated as `FILL_RESISTANCE`, and each
/// edge conductance is `2 / (r_i + r_j)`. Nothing beyond the borrowed raster,
/// the ground spec and a few scalars is stored.
pub struct StencilOperator<'a> {
    nrows: usize,
    ncols: usize,
    nodata: f64,
    resistance: &'a [f64],
    ground: GroundSpec,
    /// Regularization `1e-5 * ||L||` added to node 0, matching
    /// [`crate::circuit::laplacian::regularize_laplacian`].
    reg_diag0: f64,
}

/// Effective resistance of a cell after nodata-filling.
#[inline]
fn effective_resistance(r: f64, nodata: f64) -> f64 {
    if r == nodata || !r.is_finite() || r <= 0.0 {
        crate::raster::FILL_RESISTANCE
    } else {
        r
    }
}

/// Edge conductance between two adjacent cells, the harmonic mean of their
/// conductances: `2 / (r_i + r_j)`.
#[inline]
fn edge_conductance(ri: f64, rj: f64) -> f64 {
    2.0 / (ri + rj)
}

impl<'a> StencilOperator<'a> {
    pub fn new(
        nrows: usize,
        ncols: usize,
        nodata: f64,
        resistance: &'a [f64],
        ground: GroundSpec,
    ) -> Self {
        assert!(nrows > 0 && ncols > 0, "Stencil requires nonempty dimensions");
        assert_eq!(resistance.len(), nrows * ncols);
        match &ground {
            GroundSpec::None => {},
            GroundSpec::Neumann(shunt) => assert_eq!(shunt.len(), resistance.len()),
            GroundSpec::Dirichlet(mask) => assert_eq!(mask.len(), resistance.len()),
        }
        let reg_diag0 = compute_reg_diag0(nrows, ncols, nodata, resistance);
        Self {
            nrows,
            ncols,
            nodata,
            resistance,
            ground,
            reg_diag0,
        }
    }

    #[cfg(feature = "instrumentation-profile")]
    pub(crate) fn ground_storage_bytes(&self) -> u64 {
        match &self.ground {
            GroundSpec::None => 0,
            GroundSpec::Neumann(shunt) => std::mem::size_of_val(shunt.as_slice()) as u64,
            GroundSpec::Dirichlet(mask) => std::mem::size_of_val(mask.as_slice()) as u64,
        }
    }

    #[inline]
    fn is_ground(&self, idx: usize) -> bool {
        match &self.ground {
            GroundSpec::Dirichlet(g) => g[idx],
            _ => false,
        }
    }

    #[inline]
    fn conductance(&self, idx: usize, nb: usize) -> f64 {
        let ri = effective_resistance(self.resistance[idx], self.nodata);
        let rj = effective_resistance(self.resistance[nb], self.nodata);
        edge_conductance(ri, rj)
    }

    /// Final diagonal `L[idx, idx]`, including grounds and regularization.
    #[inline]
    fn diagonal(&self, idx: usize) -> f64 {
        if self.is_ground(idx) {
            // Dirichlet: identity row.
            return 1.0;
        }
        let mut d = self.sum_neighbour_conductance(idx);
        if let GroundSpec::Neumann(shunt) = &self.ground {
            d += shunt[idx];
        }
        if idx == 0 {
            d += self.reg_diag0;
        }
        d
    }

    /// Sum of conductances to the (up to four) orthogonal neighbours.
    #[inline]
    fn sum_neighbour_conductance(&self, idx: usize) -> f64 {
        let ncols = self.ncols;
        let r = idx / ncols;
        let c = idx % ncols;
        let mut d = 0.0;
        if r > 0 {
            d += self.conductance(idx, idx - ncols);
        }
        if r + 1 < self.nrows {
            d += self.conductance(idx, idx + ncols);
        }
        if c > 0 {
            d += self.conductance(idx, idx - 1);
        }
        if c + 1 < ncols {
            d += self.conductance(idx, idx + 1);
        }
        d
    }
}

impl<'a> Operator for StencilOperator<'a> {
    fn n(&self) -> usize {
        self.nrows * self.ncols
    }

    fn matvec(&self, x: &[f64], y: &mut [f64]) {
        let ncols = self.ncols;
        let n = self.n();
        for idx in 0..n {
            // Dirichlet: identity row, y = x.
            if self.is_ground(idx) {
                y[idx] = x[idx];
                continue;
            }

            let r = idx / ncols;
            let c = idx % ncols;
            let mut diag = 0.0;
            let mut off = 0.0;

            if r > 0 {
                let nb = idx - ncols;
                let cond = self.conductance(idx, nb);
                diag += cond;
                if !self.is_ground(nb) {
                    off += cond * x[nb];
                }
            }
            if r + 1 < self.nrows {
                let nb = idx + ncols;
                let cond = self.conductance(idx, nb);
                diag += cond;
                if !self.is_ground(nb) {
                    off += cond * x[nb];
                }
            }
            if c > 0 {
                let nb = idx - 1;
                let cond = self.conductance(idx, nb);
                diag += cond;
                if !self.is_ground(nb) {
                    off += cond * x[nb];
                }
            }
            if c + 1 < ncols {
                let nb = idx + 1;
                let cond = self.conductance(idx, nb);
                diag += cond;
                if !self.is_ground(nb) {
                    off += cond * x[nb];
                }
            }

            let mut d = diag;
            if let GroundSpec::Neumann(shunt) = &self.ground {
                d += shunt[idx];
            }
            if idx == 0 {
                d += self.reg_diag0;
            }

            y[idx] = d * x[idx] - off;
        }
    }

    fn diag_inv(&self) -> Vec<f64> {
        (0..self.n())
            .map(|idx| {
                let d = self.diagonal(idx);
                if d.abs() > 1e-15 {
                    1.0 / d.abs()
                } else {
                    0.0
                }
            })
            .collect()
    }

    fn for_each_entry(&self, row: usize, f: &mut dyn FnMut(usize, f64)) {
        if self.is_ground(row) {
            f(row, 1.0);
            return;
        }

        let ncols = self.ncols;
        let r = row / ncols;
        let c = row % ncols;
        let mut diag = 0.0;

        if r > 0 {
            let nb = row - ncols;
            let cond = self.conductance(row, nb);
            diag += cond;
            if !self.is_ground(nb) {
                f(nb, -cond);
            }
        }
        if r + 1 < self.nrows {
            let nb = row + ncols;
            let cond = self.conductance(row, nb);
            diag += cond;
            if !self.is_ground(nb) {
                f(nb, -cond);
            }
        }
        if c > 0 {
            let nb = row - 1;
            let cond = self.conductance(row, nb);
            diag += cond;
            if !self.is_ground(nb) {
                f(nb, -cond);
            }
        }
        if c + 1 < ncols {
            let nb = row + 1;
            let cond = self.conductance(row, nb);
            diag += cond;
            if !self.is_ground(nb) {
                f(nb, -cond);
            }
        }

        let mut d = diag;
        if let GroundSpec::Neumann(shunt) = &self.ground {
            d += shunt[row];
        }
        if row == 0 {
            d += self.reg_diag0;
        }
        f(row, d);
    }
}

/// Recompute the `1e-5 * ||L||` regularization that
/// [`crate::circuit::laplacian::regularize_laplacian`] applies to node 0,
/// without materialising `L`.
fn compute_reg_diag0(nrows: usize, ncols: usize, nodata: f64, resistance: &[f64]) -> f64 {
    let mut norm2 = 0.0f64;
    for idx in 0..resistance.len() {
        let r = idx / ncols;
        let c = idx % ncols;
        let ri = effective_resistance(resistance[idx], nodata);
        let mut diag = 0.0;
        if r > 0 {
            let cond = edge_conductance(ri, effective_resistance(resistance[idx - ncols], nodata));
            diag += cond;
            norm2 += cond * cond;
        }
        if r + 1 < nrows {
            let cond = edge_conductance(ri, effective_resistance(resistance[idx + ncols], nodata));
            diag += cond;
            norm2 += cond * cond;
        }
        if c > 0 {
            let cond = edge_conductance(ri, effective_resistance(resistance[idx - 1], nodata));
            diag += cond;
            norm2 += cond * cond;
        }
        if c + 1 < ncols {
            let cond = edge_conductance(ri, effective_resistance(resistance[idx + 1], nodata));
            diag += cond;
            norm2 += cond * cond;
        }
        norm2 += diag * diag;
    }

    let norm = norm2.sqrt();
    if !norm.is_finite() || norm <= 0.0 {
        0.0
    } else {
        1e-5 * norm
    }
}

/// The fine-grid Laplacian, either materialised (`Explicit`) or matrix-free
/// (`Stencil`). Implements [`Operator`] by delegation so the solver treats
/// both representations identically.
pub enum FineOperator<'a> {
    Explicit(CsMat<f64>),
    Stencil(StencilOperator<'a>),
}

impl<'a> Operator for FineOperator<'a> {
    fn n(&self) -> usize {
        match self {
            FineOperator::Explicit(m) => m.n(),
            FineOperator::Stencil(s) => s.n(),
        }
    }

    fn matvec(&self, x: &[f64], y: &mut [f64]) {
        match self {
            FineOperator::Explicit(m) => m.matvec(x, y),
            FineOperator::Stencil(s) => s.matvec(x, y),
        }
    }

    fn diag_inv(&self) -> Vec<f64> {
        match self {
            FineOperator::Explicit(m) => m.diag_inv(),
            FineOperator::Stencil(s) => s.diag_inv(),
        }
    }

    fn for_each_entry(&self, row: usize, f: &mut dyn FnMut(usize, f64)) {
        match self {
            FineOperator::Explicit(m) => m.for_each_entry(row, f),
            FineOperator::Stencil(s) => s.for_each_entry(row, f),
        }
    }
}
