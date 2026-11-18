use crate::linalg::{operator::Operator, real::{scalar, Real}, transfer::Transfer};
use sprs::CsMat;
use std::cell::RefCell;

/// Persistent scratch for P^T L (P x). Both buffers have the fine operator's
/// size and are reused across all level-1 matvecs and smoothing sweeps.
pub(super) struct GalerkinMatvecWorkspace<F: Real = f64> {
    pub(super) prolonged: Vec<F>,
    pub(super) applied: Vec<F>,
}

impl<F: Real> GalerkinMatvecWorkspace<F> {
    pub(super) fn new(n: usize) -> Self {
        Self {
            prolonged: vec![scalar::<F>(0.0); n],
            applied: vec![scalar::<F>(0.0); n],
        }
    }
}

/// Multigrid preconditioner: applies one V-cycle as `M⁻¹·r`.
/// Local accumulation of P^T L P for a five-point fine operator. A transfer
/// column spans three fine cells per axis; one fine edge extends that support
/// by one cell. The resulting coarse columns lie within one cell of the row.
/// Keep presence separate from values: cancellation must not drop structural
/// entries that the explicit symbolic Galerkin pass would retain.
pub(super) struct GalerkinRow<F: Real = f64> {
    values: [F; 9],
    present: u32,
}

pub(super) struct GalerkinOperator<'b, O: Operator<F> + ?Sized, F: Real = f64> {
    pub(super) fine: &'b O,
    pub(super) transfer: Transfer,
    pub(super) reg_diag0: F,
    pub(super) matvec_workspace: &'b RefCell<GalerkinMatvecWorkspace<F>>,
}

impl<F: Real, O: Operator<F> + ?Sized> GalerkinOperator<'_, O, F> {
    fn row(&self, row: usize) -> GalerkinRow::<F> {
        let mut result = GalerkinRow::<F> {
            values: [scalar::<F>(0.0); 9],
            present: 0,
        };
        let nc = self.transfer.coarse_ncols;
        let cr = (row / nc) as isize;
        let cc = (row % nc) as isize;
        self.transfer.for_each_column::<F>(row, |a, p_ai| {
            self.fine.for_each_entry(a, &mut |b, l_ab| {
                self.transfer.for_each_row_coordinate::<F>(b, |jr, jc, p_bj| {
                    let dr = jr as isize - cr;
                    let dc = jc as isize - cc;
                    assert!(
                        (-1..=1).contains(&dr) && (-1..=1).contains(&dc),
                        "matrix-free Galerkin requires a five-point fine operator"
                    );
                    let slot = ((dr + 1) * 3 + dc + 1) as usize;
                    result.present |= 1 << slot;
                    result.values[slot] += p_ai * l_ab * p_bj;
                });
            });
        });
        if row == 0 {
            result.values[4] += self.reg_diag0;
        }
        result
    }

    fn visit_row(&self, row: usize, entries: &GalerkinRow<F>, f: &mut dyn FnMut(usize, F)) {
        let nc = self.transfer.coarse_ncols;
        let cr = (row / nc) as isize;
        let cc = (row % nc) as isize;
        for slot in 0..9 {
            if entries.present & (1 << slot) != 0 {
                let r = cr + slot as isize / 3 - 1;
                let c = cc + slot as isize % 3 - 1;
                f(r as usize * nc + c as usize, entries.values[slot]);
            }
        }
    }

    /// Match regularize_laplacian: norm of the aggregated, unregularised
    /// entries, followed by a correction at node 0. No rows are retained.
    pub(super) fn setup_metadata(&self) -> (F, usize) {
        let mut norm2 = scalar::<F>(0.0);
        let mut nnz = 0;
        for row in 0..self.n() {
            self.for_each_entry(row, &mut |_, val| {
                norm2 += val * val;
                nnz += 1;
            });
        }
        let norm = norm2.sqrt();
        let reg = if norm.is_finite() && norm > scalar::<F>(0.0) {
            scalar::<F>(1e-5) * norm
        } else {
            scalar::<F>(0.0)
        };
        (reg, nnz)
    }
}

impl<F: Real, O: Operator<F> + ?Sized> Operator<F> for GalerkinOperator<'_, O, F> {
    fn n(&self) -> usize {
        self.transfer.coarse_n()
    }

    fn matvec(&self, x: &[F], y: &mut [F]) {
        let mut workspace = self.matvec_workspace.borrow_mut();
        let GalerkinMatvecWorkspace::<F> { prolonged, applied } = &mut *workspace;
        prolonged.fill(scalar::<F>(0.0));
        self.transfer.prolongate(x, prolonged);
        self.fine.matvec(prolonged, applied);
        self.transfer.restrict(applied, y);
        y[0] += self.reg_diag0 * x[0];
    }

    fn diag_inv(&self) -> Vec<F> {
        (0..self.n())
            .map(|row| {
                let diag = self.row(row).values[4];
                if diag.abs() > scalar::<F>(1e-15) {
                    scalar::<F>(1.0) / diag.abs()
                } else {
                    scalar::<F>(0.0)
                }
            })
            .collect()
    }

    fn for_each_entry(&self, row: usize, f: &mut dyn FnMut(usize, F)) {
        self.visit_row(row, &self.row(row), f);
    }
}

/// Galerkin coarse operator: L_coarse = P^T L_fine P. Transfer entries are
/// generated in both orientations; neither P nor L_fine P is materialised.
pub(super) fn galerkin_coarsen_operator<F: Real>(fine: &dyn Operator<F>, p: Transfer) -> CsMat::<F> {
    let coarse_n = p.coarse_n();
    // Symbolic pass: count distinct columns before allocating exact CSR slots.
    // This avoids hash maps and the potentially large intermediate L_fine P.
    let mut seen = vec![usize::MAX; coarse_n];
    let mut row_nnz = vec![0usize; coarse_n];
    for i in 0..coarse_n {
        p.for_each_column::<F>(i, |a, _| {
            fine.for_each_entry(a, &mut |b, _| {
                p.for_each_row::<F>(b, |j, _| {
                    if seen[j] != i {
                        seen[j] = i;
                        row_nnz[i] += 1;
                    }
                });
            });
        });
    }
    let mut indptr = vec![0usize; coarse_n + 1];
    for i in 0..coarse_n {
        indptr[i + 1] = indptr[i] + row_nnz[i];
    }
    let nnz = indptr[coarse_n];

    // Numeric pass: aggregate duplicate contributions into the allocated slots.
    seen.fill(usize::MAX);
    let mut pos = vec![0usize; coarse_n];
    let mut row_count = vec![0usize; coarse_n];
    let mut cols = vec![0usize; nnz];
    let mut vals = vec![scalar::<F>(0.0); nnz];
    for i in 0..coarse_n {
        p.for_each_column::<F>(i, |a, p_ai| {
            fine.for_each_entry(a, &mut |b, l_ab| {
                p.for_each_row::<F>(b, |j, p_bj| {
                    let slot = if seen[j] != i {
                        seen[j] = i;
                        let slot = indptr[i] + row_count[i];
                        row_count[i] += 1;
                        pos[j] = slot;
                        cols[slot] = j;
                        slot
                    } else {
                        pos[j]
                    };
                    vals[slot] += p_ai * l_ab * p_bj;
                });
            });
        });
    }
    CsMat::new_from_unsorted((coarse_n, coarse_n), indptr, cols, vals).unwrap()
}
