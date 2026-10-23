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

use crate::linalg::real::{Real, scalar};
use sprs::CsMat;

/// Ground boundary conditions, declared in terms of the full grid (one entry
/// per node, node index == flat cell index).
pub enum GroundSpec<'a, F: Real = f64> {
    /// No physical grounds. The usual numerical regularization is retained;
    /// the solver may separately remove the source mean to balance injection.
    None,
    /// Conductance-to-ground (Shunt): `shunt[i]` is added to `L[i,i]`.
    Shunt(Vec<F>),
    /// Original ground raster, filtered on access without allocating a copy.
    ShuntRaster(&'a [F]),
    /// Fixed-voltage (Dirichlet): `ground[i]` pins node `i` at `V = 0`.
    Dirichlet(Vec<bool>),
}

/// A linear operator representing the graph Laplacian `L`.
pub trait Operator<F: Real = f64> {
    /// Number of unknowns (nodes).
    fn n(&self) -> usize;

    /// `y = L x`.
    fn matvec(&self, x: &[F], y: &mut [F]);

    /// `1 / |L[i,i]|` per node (used by the Jacobi preconditioner).
    fn diag_inv(&self) -> Vec<F>;

    /// Invoke `f(col, val)` for every stored entry of row `row` (diagonal and
    /// off-diagonal). For a stencil these are computed on the fly.
    fn for_each_entry(&self, row: usize, f: &mut dyn FnMut(usize, F));
}

/// The explicit CSR Laplacian implements the operator directly.
impl<F: Real> Operator<F> for CsMat<F> {
    fn n(&self) -> usize {
        self.rows()
    }

    fn matvec(&self, x: &[F], y: &mut [F]) {
        crate::linalg::pcg::mat_vec_mul_slice(self, x, y);
    }

    fn diag_inv(&self) -> Vec<F> {
        crate::circuit::laplacian::extract_diag_inv(self)
    }

    fn for_each_entry(&self, row: usize, f: &mut dyn FnMut(usize, F)) {
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
pub struct StencilOperator<'a, F: Real = f64> {
    nrows: usize,
    ncols: usize,
    nodata: F,
    resistance: &'a [F],
    ground: GroundSpec<'a, F>,
    circuitscape: bool,
    /// For ground-free systems, `1e-5 * ||L||` added to node 0, matching
    /// [`crate::circuit::laplacian::regularize_laplacian`].
    reg_diag0: F,
}

/// Effective resistance of a cell after nodata-filling.
#[inline]
fn effective_resistance<F: Real>(r: F, nodata: F) -> F {
    if r == nodata || !r.is_finite() || r <= scalar::<F>(0.0) {
        scalar::<F>(crate::raster::FILL_RESISTANCE)
    } else {
        r
    }
}

/// Edge conductance between two adjacent cells, the harmonic mean of their
/// conductances: `2 / (r_i + r_j)`.
#[inline]
fn edge_conductance<F: Real>(ri: F, rj: F) -> F {
    scalar::<F>(2.0) / (ri + rj)
}

#[inline]
fn valid_raster_shunt<F: Real>(value: F, nodata: F) -> bool {
    value.is_finite() && value > scalar::<F>(0.0) && (value - nodata).abs() > scalar::<F>(1e-10)
}

impl<'a, F: Real> StencilOperator<'a, F> {
    pub fn new(
        nrows: usize,
        ncols: usize,
        nodata: F,
        resistance: &'a [F],
        ground: GroundSpec<'a, F>,
    ) -> Self {
        assert!(nrows > 0 && ncols > 0, "Stencil requires nonempty dimensions");
        assert_eq!(resistance.len(), nrows * ncols);
        match &ground {
            GroundSpec::None => {},
            GroundSpec::Shunt(shunt) => assert_eq!(shunt.len(), resistance.len()),
            GroundSpec::ShuntRaster(shunt) => assert_eq!(shunt.len(), resistance.len()),
            GroundSpec::Dirichlet(mask) => assert_eq!(mask.len(), resistance.len()),
        }
        let grounded = match &ground {
            GroundSpec::None => false,
            GroundSpec::Shunt(shunt) => shunt.iter().any(|&value| value.is_finite() && value > scalar::<F>(0.0)),
            GroundSpec::ShuntRaster(shunt) => shunt.iter().any(|&value| valid_raster_shunt(value, nodata)),
            GroundSpec::Dirichlet(mask) => mask.iter().any(|&value| value),
        };
        let reg_diag0 = if grounded {
            scalar::<F>(0.0)
        } else {
            compute_reg_diag0(nrows, ncols, nodata, resistance)
        };
        Self {
            nrows,
            ncols,
            nodata,
            resistance,
            ground,
            circuitscape: false,
            reg_diag0,
        }
    }

    /// Circuitscape's eight-neighbour raster graph, with no edges to nodata.
    /// Unused raster cells are isolated diagonal rows so multigrid can retain
    /// its rectangular indexing without introducing paths outside the habitat.
    pub fn circuitscape(
        nrows: usize, ncols: usize, nodata: F, resistance: &'a [F], ground: GroundSpec<'a, F>,
    ) -> Self {
        let mut op = Self::new(nrows, ncols, nodata, resistance, ground);
        op.circuitscape = true;
        op
    }

    #[inline]
    fn is_nodata(&self, i: usize) -> bool {
        let r = self.resistance[i];
        !r.is_finite() || r <= F::zero() || r == self.nodata
    }

    #[inline]
    fn pinned_diagonal(&self, i: usize) -> F {
        // The unused unknown stays at zero. Keep its arbitrary diagonal near
        // habitat conductances rather than overwhelming Galerkin coarse rows.
        if self.circuitscape && self.is_nodata(i) { scalar::<F>(1e-9) } else { scalar::<F>(1.0) }
    }

    #[inline]
    fn circuitscape_edges(&self, i: usize, mut f: impl FnMut(usize, f64)) {
        let row = i / self.ncols;
        let col = i % self.ncols;
        let ri = self.resistance[i].to_f64().unwrap();
        for dr in -1_isize..=1 {
            for dc in -1_isize..=1 {
                if dr == 0 && dc == 0 { continue; }
                let r = row as isize + dr;
                let c = col as isize + dc;
                if r < 0 || c < 0 || r >= self.nrows as isize || c >= self.ncols as isize { continue; }
                let j = r as usize * self.ncols + c as usize;
                if self.is_nodata(j) { continue; }
                let distance = if dr != 0 && dc != 0 { std::f64::consts::SQRT_2 } else { 1.0 };
                f(j, 2.0 / ((ri + self.resistance[j].to_f64().unwrap()) * distance));
            }
        }
    }

    /// Accurate equation row from the stored raster values, without f32 cancellation.
    pub(crate) fn row_f64<X: Real>(&self, i: usize, x: &[X]) -> (f64, f64) {
        if self.is_ground(i) {
            let diagonal = self.pinned_diagonal(i).to_f64().unwrap();
            return (diagonal * x[i].to_f64().unwrap(), diagonal);
        }
        let vi = x[i].to_f64().unwrap();
        let ri = effective_resistance(self.resistance[i], self.nodata).to_f64().unwrap();
        let mut diagonal = self.shunt_conductance(i).to_f64().unwrap();
        if i == 0 { diagonal += self.reg_diag0.to_f64().unwrap(); }
        let mut value = diagonal * vi;
        if self.circuitscape {
            self.circuitscape_edges(i, |j, conductance| {
                let vj = if self.is_ground(j) { 0.0 } else { x[j].to_f64().unwrap() };
                value += conductance * (vi - vj);
                diagonal += conductance;
            });
            return (value, diagonal);
        }
        let r = i / self.ncols;
        let c = i % self.ncols;
        for j in [
            (r > 0).then(|| i - self.ncols),
            (r + 1 < self.nrows).then(|| i + self.ncols),
            (c > 0).then(|| i - 1),
            (c + 1 < self.ncols).then(|| i + 1),
        ].into_iter().flatten() {
            let rj = effective_resistance(self.resistance[j], self.nodata).to_f64().unwrap();
            let conductance = 2.0 / (ri + rj);
            let vj = if self.is_ground(j) { 0.0 } else { x[j].to_f64().unwrap() };
            value += conductance * (vi - vj);
            diagonal += conductance;
        }
        (value, diagonal)
    }

    #[cfg(feature = "instrumentation-profile")]
    pub(crate) fn ground_storage_bytes(&self) -> u64 {
        match &self.ground {
            GroundSpec::None => 0,
            GroundSpec::Shunt(shunt) => std::mem::size_of_val(shunt.as_slice()) as u64,
            GroundSpec::ShuntRaster(_) => 0,
            GroundSpec::Dirichlet(mask) => std::mem::size_of_val(mask.as_slice()) as u64,
        }
    }

    #[inline]
    fn is_ground(&self, idx: usize) -> bool {
        if self.circuitscape && self.is_nodata(idx) { return true; }
        match &self.ground {
            GroundSpec::Dirichlet(g) => g[idx],
            _ => false,
        }
    }

    #[inline]
    fn shunt_conductance(&self, idx: usize) -> F {
        match &self.ground {
            GroundSpec::Shunt(shunt) => shunt[idx],
            GroundSpec::ShuntRaster(shunt) => {
                let value = shunt[idx];
                if valid_raster_shunt(value, self.nodata) { value } else { scalar::<F>(0.0) }
            }
            _ => scalar::<F>(0.0),
        }
    }

    #[inline]
    fn conductance(&self, idx: usize, nb: usize) -> F {
        let ri = effective_resistance(self.resistance[idx], self.nodata);
        let rj = effective_resistance(self.resistance[nb], self.nodata);
        edge_conductance(ri, rj)
    }

    /// Final diagonal `L[idx, idx]`, including grounds and regularization.
    #[inline]
    fn diagonal(&self, idx: usize) -> F {
        if self.is_ground(idx) {
            // Dirichlet: identity row.
            return self.pinned_diagonal(idx);
        }
        let mut d = self.sum_neighbour_conductance(idx);
        d += self.shunt_conductance(idx);
        if idx == 0 {
            d += self.reg_diag0;
        }
        d
    }

    /// Sum of conductances to the (up to four) orthogonal neighbours.
    #[inline]
    fn sum_neighbour_conductance(&self, idx: usize) -> F {
        if self.circuitscape {
            let mut sum = F::zero();
            self.circuitscape_edges(idx, |_, g| sum += scalar::<F>(g));
            return sum;
        }
        let ncols = self.ncols;
        let r = idx / ncols;
        let c = idx % ncols;
        let mut d = scalar::<F>(0.0);
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

impl<'a, F: Real> Operator<F> for StencilOperator<'a, F> {
    fn n(&self) -> usize {
        self.nrows * self.ncols
    }

    fn matvec(&self, x: &[F], y: &mut [F]) {
        if self.circuitscape {
            for idx in 0..self.n() {
                let mut value = F::zero();
                self.for_each_entry(idx, &mut |j, a| value += a * x[j]);
                y[idx] = value;
            }
            return;
        }
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
            let mut diag = scalar::<F>(0.0);
            let mut off = scalar::<F>(0.0);

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
            d += self.shunt_conductance(idx);
            if idx == 0 {
                d += self.reg_diag0;
            }

            y[idx] = d * x[idx] - off;
        }
    }

    fn diag_inv(&self) -> Vec<F> {
        (0..self.n())
            .map(|idx| {
                let d = self.diagonal(idx);
                if d.abs() > scalar::<F>(1e-15) {
                    scalar::<F>(1.0) / d.abs()
                } else {
                    scalar::<F>(0.0)
                }
            })
            .collect()
    }

    fn for_each_entry(&self, row: usize, f: &mut dyn FnMut(usize, F)) {
        if self.is_ground(row) {
            f(row, self.pinned_diagonal(row));
            return;
        }

        if self.circuitscape {
            let mut diagonal = self.shunt_conductance(row);
            if row == 0 { diagonal += self.reg_diag0; }
            self.circuitscape_edges(row, |j, g| {
                let g = scalar::<F>(g);
                diagonal += g;
                if !self.is_ground(j) { f(j, -g); }
            });
            f(row, diagonal);
            return;
        }

        let ncols = self.ncols;
        let r = row / ncols;
        let c = row % ncols;
        let mut diag = scalar::<F>(0.0);

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
        d += self.shunt_conductance(row);
        if row == 0 {
            d += self.reg_diag0;
        }
        f(row, d);
    }
}

/// Recompute the `1e-5 * ||L||` regularization that
/// [`crate::circuit::laplacian::regularize_laplacian`] applies to node 0,
/// without materialising `L`.
fn compute_reg_diag0<F: Real>(nrows: usize, ncols: usize, nodata: F, resistance: &[F]) -> F {
    let mut norm2 = scalar::<F>(0.0);
    for idx in 0..resistance.len() {
        let r = idx / ncols;
        let c = idx % ncols;
        let ri = effective_resistance(resistance[idx], nodata);
        let mut diag = scalar::<F>(0.0);
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
    if !norm.is_finite() || norm <= scalar::<F>(0.0) {
        scalar::<F>(0.0)
    } else {
        scalar::<F>(1e-5) * norm
    }
}

/// The fine-grid Laplacian, either materialised (`Explicit`) or matrix-free
/// (`Stencil`). Implements [`Operator`] by delegation so the solver treats
/// both representations identically.
pub enum FineOperator<'a, F: Real = f64> {
    Explicit(CsMat<F>),
    Stencil(StencilOperator<'a, F>),
}

impl<'a, F: Real> Operator<F> for FineOperator<'a, F> {
    fn n(&self) -> usize {
        match self {
            FineOperator::Explicit(m) => m.n(),
            FineOperator::Stencil(s) => s.n(),
        }
    }

    fn matvec(&self, x: &[F], y: &mut [F]) {
        match self {
            FineOperator::Explicit(m) => m.matvec(x, y),
            FineOperator::Stencil(s) => s.matvec(x, y),
        }
    }

    fn diag_inv(&self) -> Vec<F> {
        match self {
            FineOperator::Explicit(m) => m.diag_inv(),
            FineOperator::Stencil(s) => s.diag_inv(),
        }
    }

    fn for_each_entry(&self, row: usize, f: &mut dyn FnMut(usize, F)) {
        match self {
            FineOperator::Explicit(m) => m.for_each_entry(row, f),
            FineOperator::Stencil(s) => s.for_each_entry(row, f),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_ground_raster_matches_prepared_shunts() {
        let resistance = vec![2.0; 12];
        let input: Vec<f64> = (0..12).map(|i| (i as f64 * 0.7).sin()).collect();
        for nodata in [-9999.0, 5.0, f64::NAN] {
            for has_ground in [false, true] {
                let mut ground = vec![0.0, -2.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY,
                    nodata, nodata + 5e-11, 0.0, 0.0, 0.0, 0.0, 0.0];
                if has_ground { ground[11] = 3.0; }
                let prepared = ground.iter().map(|&value| {
                    if valid_raster_shunt(value, nodata) { value } else { 0.0 }
                }).collect();
                let expected = StencilOperator::new(3, 4, nodata, &resistance, GroundSpec::Shunt(prepared));
                let actual = StencilOperator::new(3, 4, nodata, &resistance, GroundSpec::ShuntRaster(&ground));
                let (mut a, mut b) = (vec![0.0; 12], vec![0.0; 12]);
                expected.matvec(&input, &mut a);
                actual.matvec(&input, &mut b);
                assert_eq!(a, b);
                assert_eq!(expected.diag_inv(), actual.diag_inv());
                for row in 0..12 {
                    let (mut a, mut b) = (Vec::new(), Vec::new());
                    expected.for_each_entry(row, &mut |col, value| a.push((col, value)));
                    actual.for_each_entry(row, &mut |col, value| b.push((col, value)));
                    assert_eq!(a, b);
                }
                #[cfg(feature = "instrumentation-profile")]
                assert_eq!(actual.ground_storage_bytes(), 0);
            }
        }
    }
}
