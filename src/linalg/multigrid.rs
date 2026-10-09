use crate::linalg::cholesky;
use crate::linalg::operator::{FineOperator, Operator};
use crate::linalg::pcg::Preconditioner;
use crate::linalg::transfer::Transfer;
use crate::memory;
#[cfg(feature = "instrumentation-profile")]
use crate::memory::GalerkinScratch;
use sprs::CsMat;
use std::cell::RefCell;

// 7×7 is the largest square fine grid below the default 8×8 coarsening threshold.
// Bound dense allocation when a thin or depth-limited grid has no coarse level.
const SMALL_FINE_DIRECT_LIMIT: usize = 49;

/// Native multigrid setup controls; browser calls use the defaults.
#[derive(Clone, Copy, Debug)]
pub struct MgOptions {
    /// Maximum hierarchy depth, including the fine level.
    pub max_levels: usize,
    /// Desired upper bound on unknowns at the direct Cholesky level.
    /// None preserves the minimum-four-cells-per-side stopping rule.
    /// A supplied size allows coarsening to smaller grids, subject to max_levels
    /// and both dimensions remaining nonzero. Inspect the profile for the
    /// effective coarsest size if those limits prevent reaching the target.
    pub cholesky_size: Option<usize>,
}

impl Default for MgOptions {
    fn default() -> Self {
        Self {
            max_levels: 8,
            cholesky_size: None,
        }
    }
}

/// One (coarse) level of the multigrid hierarchy.
struct MgLevel {
    laplacian: CoarseOperator,
    nrows: usize,
    ncols: usize,
    cholesky_l: Option<Vec<f64>>,
    /// Geometric transfer from this level to the finer level above.
    prolongation: Transfer,
}

/// Per-level scratch vectors used by the V-cycle:
/// `e` (error approximation), `d_prime` (residual of the error system),
/// `d` (the level's right-hand side).
struct LevelWorkspace {
    e: Vec<f64>,
    d_prime: Vec<f64>,
    d: Vec<f64>,
    smoother: Smoother,
}

/// Coefficients are cached only for the diagonal-based smoother. Explicit MG
/// retains its existing Gauss–Seidel update and damping.
enum Smoother {
    SymmetricGaussSeidel,
    WeightedJacobi { diag_inv: Vec<f64>, omega: f64 },
}

impl Smoother {
    fn weighted_jacobi(op: &dyn Operator, requested_omega: f64) -> Self {
        let mut diag_inv = vec![0.0; op.n()];
        let mut upper_bound: f64 = 0.0;
        for (row, inverse) in diag_inv.iter_mut().enumerate() {
            let mut diagonal: f64 = 0.0;
            let mut abs_sum: f64 = 0.0;
            op.for_each_entry(row, &mut |col, value| {
                abs_sum += value.abs();
                if col == row {
                    diagonal = value;
                }
            });
            if diagonal.abs() > 1e-15 {
                *inverse = 1.0 / diagonal.abs();
                upper_bound = upper_bound.max(abs_sum * *inverse);
            }
        }
        // Gershgorin bounds the eigenvalues of D^-1 L by the largest absolute
        // row sum. Keep omega*lambda_max <= 1.8 < 2 for damped Jacobi.
        let omega = requested_omega.min(1.8 / upper_bound.max(1.0));
        Self::WeightedJacobi { diag_inv, omega }
    }

    fn smooth(
        &self,
        op: &dyn Operator,
        e: &mut [f64],
        d: &[f64],
        scratch: &mut [f64],
        gs_omega: f64,
    ) {
        match self {
            Self::SymmetricGaussSeidel => symmetric_gauss_seidel_smooth(op, e, d, gs_omega),
            Self::WeightedJacobi { diag_inv, omega } => {
                weighted_jacobi_smooth(op, e, d, diag_inv, *omega, scratch)
            }
        }
    }
}

/// Persistent scratch for P^T L (P x). Both buffers have the fine operator's
/// size and are reused across all level-1 matvecs and smoothing sweeps.
struct GalerkinMatvecWorkspace {
    prolonged: Vec<f64>,
    applied: Vec<f64>,
}

impl GalerkinMatvecWorkspace {
    fn new(n: usize) -> Self {
        Self {
            prolonged: vec![0.0; n],
            applied: vec![0.0; n],
        }
    }
}

/// Multigrid preconditioner: applies one V-cycle as `M⁻¹·r`.
///
/// Level 0 (the fine grid) is an [`Operator`]: either a materialised CSR
/// matrix or a matrix-free stencil. The stencil path uses weighted Jacobi
/// and applies level 1 as P^T L_0 (P x); level 2 is the first CSR. Explicit MG
/// stores every Laplacian and uses symmetric Gauss–Seidel. Both paths use
/// geometric transfers without entry arrays.
pub struct MgPreconditioner<'a> {
    fine: FineOperator<'a>,
    // Retain operators for post-smoothing on ascent and reuse across PCG iterations.
    coarse: Vec<MgLevel>,
    fine_cholesky_l: Option<Vec<f64>>,
    nu: usize,
    omega: f64,
    // Preconditioner::apply takes &self; RefCell permits scratch-buffer reuse.
    // Borrow once at the boundary, then recurse with ordinary mutable slices.
    workspaces: RefCell<Vec<LevelWorkspace>>,
}

/// Level 1 stores only its regularisation and structure count; deeper levels
/// retain CSR matrices. The fine operator is borrowed only by temporary views,
/// so the owning hierarchy never contains references into itself.
enum CoarseOperator {
    Galerkin {
        reg_diag0: f64,
        matvec_workspace: RefCell<GalerkinMatvecWorkspace>,
        #[cfg(feature = "instrumentation-profile")]
        nnz: usize,
    },
    Explicit(CsMat<f64>),
}

impl MgLevel {
    fn operator<'b>(&'b self, fine: &'b FineOperator<'b>) -> LevelOperator<'b> {
        match &self.laplacian {
            CoarseOperator::Explicit(matrix) => LevelOperator::Borrowed(matrix),
            CoarseOperator::Galerkin {
                reg_diag0,
                matvec_workspace,
                ..
            } => LevelOperator::Galerkin(GalerkinOperator {
                fine,
                transfer: self.prolongation,
                reg_diag0: *reg_diag0,
                matvec_workspace,
            }),
        }
    }

    #[cfg(feature = "instrumentation-profile")]
    fn nnz(&self) -> usize {
        match &self.laplacian {
            CoarseOperator::Explicit(matrix) => matrix.nnz(),
            CoarseOperator::Galerkin { nnz, .. } => *nnz,
        }
    }
}

/// A borrowed view of any hierarchy level.
enum LevelOperator<'b> {
    Borrowed(&'b dyn Operator),
    Galerkin(GalerkinOperator<'b, FineOperator<'b>>),
}

impl Operator for LevelOperator<'_> {
    fn n(&self) -> usize {
        match self {
            Self::Borrowed(op) => op.n(),
            Self::Galerkin(op) => op.n(),
        }
    }
    fn matvec(&self, x: &[f64], y: &mut [f64]) {
        match self {
            Self::Borrowed(op) => op.matvec(x, y),
            Self::Galerkin(op) => op.matvec(x, y),
        }
    }
    fn diag_inv(&self) -> Vec<f64> {
        match self {
            Self::Borrowed(op) => op.diag_inv(),
            Self::Galerkin(op) => op.diag_inv(),
        }
    }
    fn for_each_entry(&self, row: usize, f: &mut dyn FnMut(usize, f64)) {
        match self {
            Self::Borrowed(op) => op.for_each_entry(row, f),
            Self::Galerkin(op) => op.for_each_entry(row, f),
        }
    }
}

/// Local accumulation of P^T L P for a five-point fine operator. A transfer
/// column spans four fine cells per axis; one fine edge extends that support
/// by one cell. The resulting coarse columns lie within two cells of the row.
/// Keep presence separate from values: cancellation must not drop structural
/// entries that the explicit symbolic Galerkin pass would retain.
struct GalerkinRow {
    values: [f64; 25],
    present: u32,
}

struct GalerkinOperator<'b, T: Operator + ?Sized> {
    fine: &'b T,
    transfer: Transfer,
    reg_diag0: f64,
    matvec_workspace: &'b RefCell<GalerkinMatvecWorkspace>,
}

impl<T: Operator + ?Sized> GalerkinOperator<'_, T> {
    fn row(&self, row: usize) -> GalerkinRow {
        let mut result = GalerkinRow {
            values: [0.0; 25],
            present: 0,
        };
        let nc = self.transfer.coarse_ncols;
        let cr = (row / nc) as isize;
        let cc = (row % nc) as isize;
        self.transfer.for_each_column(row, |a, p_ai| {
            self.fine.for_each_entry(a, &mut |b, l_ab| {
                self.transfer.for_each_row_coordinate(b, |jr, jc, p_bj| {
                    let dr = jr as isize - cr;
                    let dc = jc as isize - cc;
                    assert!(
                        (-2..=2).contains(&dr) && (-2..=2).contains(&dc),
                        "matrix-free Galerkin requires a five-point fine operator"
                    );
                    let slot = ((dr + 2) * 5 + dc + 2) as usize;
                    result.present |= 1 << slot;
                    result.values[slot] += p_ai * l_ab * p_bj;
                });
            });
        });
        if row == 0 {
            result.values[12] += self.reg_diag0;
        }
        result
    }

    fn visit_row(&self, row: usize, entries: &GalerkinRow, f: &mut dyn FnMut(usize, f64)) {
        let nc = self.transfer.coarse_ncols;
        let cr = (row / nc) as isize;
        let cc = (row % nc) as isize;
        for slot in 0..25 {
            if entries.present & (1 << slot) != 0 {
                let r = cr + slot as isize / 5 - 2;
                let c = cc + slot as isize % 5 - 2;
                f(r as usize * nc + c as usize, entries.values[slot]);
            }
        }
    }

    /// Match regularize_laplacian: norm of the aggregated, unregularised
    /// entries, followed by a correction at node 0. No rows are retained.
    fn setup_metadata(&self) -> (f64, usize) {
        let mut norm2 = 0.0;
        let mut nnz = 0;
        for row in 0..self.n() {
            self.for_each_entry(row, &mut |_, val| {
                norm2 += val * val;
                nnz += 1;
            });
        }
        let norm = norm2.sqrt();
        let reg = if norm.is_finite() && norm > 0.0 {
            1e-5 * norm
        } else {
            0.0
        };
        (reg, nnz)
    }
}

impl<T: Operator + ?Sized> Operator for GalerkinOperator<'_, T> {
    fn n(&self) -> usize {
        self.transfer.coarse_n()
    }

    fn matvec(&self, x: &[f64], y: &mut [f64]) {
        let mut workspace = self.matvec_workspace.borrow_mut();
        let GalerkinMatvecWorkspace { prolonged, applied } = &mut *workspace;
        prolonged.fill(0.0);
        self.transfer.prolongate(x, prolonged);
        self.fine.matvec(prolonged, applied);
        self.transfer.restrict(applied, y);
        y[0] += self.reg_diag0 * x[0];
    }

    fn diag_inv(&self) -> Vec<f64> {
        (0..self.n())
            .map(|row| {
                let diag = self.row(row).values[12];
                if diag.abs() > 1e-15 {
                    1.0 / diag.abs()
                } else {
                    0.0
                }
            })
            .collect()
    }

    fn for_each_entry(&self, row: usize, f: &mut dyn FnMut(usize, f64)) {
        self.visit_row(row, &self.row(row), f);
    }
}

/// Galerkin coarse operator: L_coarse = P^T L_fine P. Transfer entries are
/// generated in both orientations; neither P nor L_fine P is materialised.
fn galerkin_coarsen_operator(fine: &dyn Operator, p: Transfer) -> CsMat<f64> {
    let coarse_n = p.coarse_n();
    // Symbolic pass: count distinct columns before allocating exact CSR slots.
    // This avoids hash maps and the potentially large intermediate L_fine P.
    let mut seen = vec![usize::MAX; coarse_n];
    let mut row_nnz = vec![0usize; coarse_n];
    for i in 0..coarse_n {
        p.for_each_column(i, |a, _| {
            fine.for_each_entry(a, &mut |b, _| {
                p.for_each_row(b, |j, _| {
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
    let mut vals = vec![0.0f64; nnz];
    for i in 0..coarse_n {
        p.for_each_column(i, |a, p_ai| {
            fine.for_each_entry(a, &mut |b, l_ab| {
                p.for_each_row(b, |j, p_bj| {
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

fn operator_to_dense(op: &dyn Operator) -> Vec<f64> {
    let n = op.n();
    let mut dense = vec![0.0; n * n];
    for row in 0..n {
        op.for_each_entry(row, &mut |col, val| dense[row * n + col] = val);
    }
    dense
}

impl MgPreconditioner<'static> {
    /// Build a multigrid hierarchy from a materialised fine-grid matrix.
    pub fn build_from_laplacian(
        laplacian: CsMat<f64>,
        nrows: usize,
        ncols: usize,
        max_levels: usize,
    ) -> Self {
        Self::build_from_operator(FineOperator::Explicit(laplacian), nrows, ncols, max_levels)
    }
}

impl<'a> MgPreconditioner<'a> {
    /// Build a multigrid hierarchy from a fine-grid operator (materialised or
    /// matrix-free). A stencil fine grid also keeps level 1 matrix-free;
    /// subsequent levels are Galerkin-coarsened and materialised.
    pub fn build_from_operator(
        fine: FineOperator<'a>,
        nrows: usize,
        ncols: usize,
        max_levels: usize,
    ) -> Self {
        Self::build_with_options(
            fine,
            nrows,
            ncols,
            MgOptions {
                max_levels,
                ..MgOptions::default()
            },
        )
    }

    /// Build with an optional direct-solve grid size. Rejects zero dimensions,
    /// zero hierarchy depth and a zero Cholesky size.
    pub fn build_with_options(
        fine: FineOperator<'a>,
        nrows: usize,
        ncols: usize,
        options: MgOptions,
    ) -> Self {
        assert!(nrows > 0 && ncols > 0, "MG requires nonempty dimensions");
        assert!(options.max_levels > 0, "MG requires at least one level");
        assert!(
            options.cholesky_size != Some(0),
            "Cholesky size must be positive"
        );
        assert_eq!(
            fine.n(),
            nrows * ncols,
            "MG hierarchy expects all cells as nodes, got {} vs {}",
            fine.n(),
            nrows * ncols
        );

        let mut coarse: Vec<MgLevel> = Vec::new();
        let (mut nr, mut nc) = (nrows, ncols);

        while 1 + coarse.len() < options.max_levels {
            if options.cholesky_size.is_some_and(|size| nr * nc <= size) {
                break;
            }
            let fine_nr = nr;
            let fine_nc = nc;
            let next_nr = fine_nr / 2;
            let next_nc = fine_nc / 2;
            let min_side = if options.cholesky_size.is_some() {
                1
            } else {
                4
            };
            if next_nr < min_side || next_nc < min_side {
                break;
            }

            let prolongation = Transfer {
                fine_nrows: fine_nr,
                fine_ncols: fine_nc,
                coarse_nrows: next_nr,
                coarse_ncols: next_nc,
            };
            let laplacian = if coarse.is_empty() && matches!(&fine, FineOperator::Stencil(_)) {
                // Keep the first Galerkin level implicit only when its source
                // is the five-point stencil; arbitrary explicit matrices need CSR.
                let matvec_workspace = RefCell::new(GalerkinMatvecWorkspace::new(fine.n()));
                let op = GalerkinOperator {
                    fine: &fine,
                    transfer: prolongation,
                    reg_diag0: 0.0,
                    matvec_workspace: &matvec_workspace,
                };
                let (reg_diag0, _nnz) = op.setup_metadata();
                CoarseOperator::Galerkin {
                    reg_diag0,
                    matvec_workspace,
                    #[cfg(feature = "instrumentation-profile")]
                    nnz: _nnz,
                }
            } else {
                // A borrowed view can read either the implicit level 1 or a
                // retained CSR, without creating an intermediate fine matrix.
                let previous = coarse
                    .last()
                    .map(|level| level.operator(&fine))
                    .unwrap_or(LevelOperator::Borrowed(&fine));
                let mut matrix = galerkin_coarsen_operator(&previous, prolongation);
                crate::circuit::laplacian::regularize_laplacian(&mut matrix);
                CoarseOperator::Explicit(matrix)
            };
            coarse.push(MgLevel {
                laplacian,
                nrows: next_nr,
                ncols: next_nc,
                cholesky_l: None,
                prolongation,
            });

            nr = next_nr;
            nc = next_nc;
        }

        // Factor once during setup, then reuse for each V-cycle. If there is
        // no coarse level, only materialise a bounded small fine-grid system.
        let mut fine_cholesky_l = None;
        if let Some(coarsest) = coarse.last_mut() {
            let cnodes = coarsest.nrows * coarsest.ncols;
            memory::record_coarse_dense(cnodes);
            let dense = operator_to_dense(&coarsest.operator(&fine));
            coarsest.cholesky_l = cholesky::cholesky_decompose(&dense, cnodes);
        } else if fine.n() <= options.cholesky_size.unwrap_or(SMALL_FINE_DIRECT_LIMIT) {
            let n = fine.n();
            memory::record_coarse_dense(n);
            let dense = operator_to_dense(&fine);
            fine_cholesky_l = cholesky::cholesky_decompose(&dense, n);
        }
        memory::record_fine_operator(&fine);
        memory::record_coarsest(
            nr * nc,
            options.cholesky_size,
            coarse
                .last()
                .map(|level| level.cholesky_l.is_some())
                .unwrap_or(fine_cholesky_l.is_some()),
        );

        #[cfg(feature = "instrumentation-profile")]
        {
            memory::record_fine_level(
                fine_cholesky_l
                    .as_ref()
                    .map(|factor| memory::vec_f64_bytes(factor.len()))
                    .unwrap_or(0),
            );
            for (k, lvl) in coarse.iter().enumerate() {
                let nodes = lvl.nrows * lvl.ncols;
                let (lap_bytes, repr, scratch) = match &lvl.laplacian {
                    CoarseOperator::Galerkin { .. } => (
                        0,
                        "galerkin-stencil",
                        GalerkinScratch {
                            row_accumulator_bytes: std::mem::size_of::<GalerkinRow>() as u64,
                            ..GalerkinScratch::default()
                        },
                    ),
                    CoarseOperator::Explicit(matrix) => (
                        memory::csmat_bytes(matrix),
                        "explicit",
                        GalerkinScratch {
                            symbolic_seen_bytes: memory::vec_usize_bytes(nodes),
                            symbolic_row_nnz_bytes: memory::vec_usize_bytes(nodes),
                            numeric_pos_bytes: memory::vec_usize_bytes(nodes),
                            numeric_row_count_bytes: memory::vec_usize_bytes(nodes),
                            row_accumulator_bytes: if k == 1
                                && matches!(&coarse[0].laplacian, CoarseOperator::Galerkin { .. })
                            {
                                std::mem::size_of::<GalerkinRow>() as u64
                            } else {
                                0
                            },
                            ..GalerkinScratch::default()
                        },
                    ),
                };
                let cholesky_bytes = lvl
                    .cholesky_l
                    .as_ref()
                    .map(|l| memory::vec_f64_bytes(l.len()))
                    .unwrap_or(0);
                memory::record_level(
                    k + 1,
                    nodes,
                    lvl.nnz(),
                    lap_bytes,
                    0,
                    cholesky_bytes,
                    scratch,
                );
                memory::record_level_representation(k + 1, repr);
            }
        }

        // Cache smoother data during setup; no allocations during a sweep.
        let low_memory = matches!(&fine, FineOperator::Stencil(_));
        let mut workspaces = Vec::with_capacity(1 + coarse.len());
        for level in 0..=coarse.len() {
            let op = if level == 0 {
                LevelOperator::Borrowed(&fine)
            } else {
                coarse[level - 1].operator(&fine)
            };
            let n = op.n();
            let smoother = if low_memory && level < coarse.len() {
                Smoother::weighted_jacobi(&op, 0.67)
            } else {
                Smoother::SymmetricGaussSeidel
            };
            #[cfg(feature = "instrumentation-profile")]
            {
                let (repr, diag_bytes, omega) = if level == coarse.len() {
                    ("none", 0, None)
                } else {
                    match &smoother {
                        Smoother::SymmetricGaussSeidel => ("symmetric-gauss-seidel", 0, Some(0.67)),
                        Smoother::WeightedJacobi { diag_inv, omega } => (
                            "weighted-jacobi",
                            memory::vec_f64_bytes(diag_inv.len()),
                            Some(*omega),
                        ),
                    }
                };
                let matvec_bytes = if level > 0 {
                    match &coarse[level - 1].laplacian {
                        CoarseOperator::Galerkin {
                            matvec_workspace, ..
                        } => {
                            let scratch = matvec_workspace.borrow();
                            memory::vec_f64_bytes(scratch.prolonged.len() + scratch.applied.len())
                        }
                        CoarseOperator::Explicit(_) => 0,
                    }
                } else {
                    0
                };
                memory::record_level_runtime(level, repr, diag_bytes, omega, matvec_bytes);
            }
            workspaces.push(LevelWorkspace {
                e: vec![0.0; n],
                d_prime: vec![0.0; n],
                d: vec![0.0; n],
                smoother,
            });
        }

        Self {
            fine,
            coarse,
            fine_cholesky_l,
            nu: 2,
            omega: 0.67,
            workspaces: RefCell::new(workspaces),
        }
    }

    /// The fine-grid operator (level 0).
    pub fn fine_operator(&self) -> &dyn Operator {
        &self.fine
    }

    fn level_operator(&self, level: usize) -> LevelOperator<'_> {
        if level == 0 {
            LevelOperator::Borrowed(&self.fine)
        } else {
            self.coarse[level - 1].operator(&self.fine)
        }
    }

    fn v_cycle(&self, workspaces: &mut [LevelWorkspace], level: usize) {
        let (current, deeper) = workspaces.split_first_mut().unwrap();
        let op = self.level_operator(level);
        current.e.fill(0.0);

        if deeper.is_empty() {
            let factor = if level == 0 {
                self.fine_cholesky_l.as_ref()
            } else {
                self.coarse[level - 1].cholesky_l.as_ref()
            };
            if let Some(factor) = factor {
                let error = cholesky::cholesky_solve(factor, &current.d, current.e.len());
                current.e.copy_from_slice(&error);
            } else {
                // Start from zero so the result does not depend on an earlier
                // application when Cholesky is unavailable.
                let result = crate::linalg::pcg::cg_solve(&op, &current.d, 50, 1e-3, None);
                current.e.copy_from_slice(&result.v);
            }
            return;
        }

        for _ in 0..self.nu {
            current.smoother.smooth(
                &op,
                &mut current.e,
                &current.d,
                &mut current.d_prime,
                self.omega,
            );
        }
        op.matvec(&current.e, &mut current.d_prime);
        for (residual, &source) in current.d_prime.iter_mut().zip(&current.d) {
            *residual = source - *residual;
        }

        let transfer = self.coarse[level].prolongation;
        transfer.restrict(&current.d_prime, &mut deeper[0].d);
        self.v_cycle(deeper, level + 1);
        transfer.prolongate(&deeper[0].e, &mut current.e);

        for _ in 0..self.nu {
            current.smoother.smooth(
                &op,
                &mut current.e,
                &current.d,
                &mut current.d_prime,
                self.omega,
            );
        }
    }
}

impl<'a> Preconditioner for MgPreconditioner<'a> {
    fn apply(&self, r: &[f64], e0: &mut Vec<f64>) {
        assert_eq!(r.len(), self.fine.n());
        let mut workspaces = self.workspaces.borrow_mut();
        workspaces[0].d.copy_from_slice(r);
        self.v_cycle(&mut workspaces, 0);
        e0.resize(r.len(), 0.0);
        e0.copy_from_slice(&workspaces[0].e);
    }
}

/// One simultaneous weighted Jacobi sweep. The entire product uses the old e;
/// only after that product is complete are any entries of e updated.
fn weighted_jacobi_smooth(
    op: &dyn Operator,
    e: &mut [f64],
    d: &[f64],
    diag_inv: &[f64],
    omega: f64,
    scratch: &mut [f64],
) {
    op.matvec(e, scratch);
    for i in 0..e.len() {
        e[i] += omega * diag_inv[i] * (d[i] - scratch[i]);
    }
}

fn symmetric_gauss_seidel_smooth(op: &dyn Operator, e: &mut [f64], d: &[f64], omega: f64) {
    let n = d.len();
    // Forward sweep
    for row in 0..n {
        let mut diag = 0.0;
        let mut off_diag_sum = 0.0;
        op.for_each_entry(row, &mut |col, val| {
            if col == row {
                diag = val;
            } else {
                off_diag_sum += val * e[col];
            }
        });
        if diag.abs() > 1e-15 {
            let new_e = (d[row] - off_diag_sum) / diag;
            e[row] = (1.0 - omega) * e[row] + omega * new_e;
        }
    }
    // Backward sweep
    for row in (0..n).rev() {
        let mut diag = 0.0;
        let mut off_diag_sum = 0.0;
        op.for_each_entry(row, &mut |col, val| {
            if col == row {
                diag = val;
            } else {
                off_diag_sum += val * e[col];
            }
        });
        if diag.abs() > 1e-15 {
            let new_e = (d[row] - off_diag_sum) / diag;
            e[row] = (1.0 - omega) * e[row] + omega * new_e;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linalg::pcg;

    // Helper to generate safe mock resistance data for a uniform grid
    fn generate_mock_resistance(nrows: usize, ncols: usize) -> Vec<f64> {
        vec![1.0; nrows * ncols]
    }

    // Build a bilinear MG hierarchy from a resistance raster (nodata -1.0).
    fn build_mg(
        resistance: &[f64],
        nrows: usize,
        ncols: usize,
        max_levels: usize,
    ) -> MgPreconditioner<'static> {
        let filled = crate::raster::fill_nodata(resistance, -1.0);
        let (_cell_to_node, _num_nodes, _edges, laplacian) =
            crate::build_circuit_model(&filled, nrows, ncols, -1.0);
        MgPreconditioner::build_from_laplacian(laplacian, nrows, ncols, max_levels)
    }

    // The tests build the explicit hierarchy, so the fine operator is a CsMat.
    fn fine_csmat<'a>(mg: &'a MgPreconditioner<'_>) -> &'a CsMat<f64> {
        match &mg.fine {
            FineOperator::Explicit(m) => m,
            FineOperator::Stencil(_) => panic!("expected an explicit fine operator"),
        }
    }

    fn num_levels(mg: &MgPreconditioner<'_>) -> usize {
        1 + mg.coarse.len()
    }

    fn level_n(mg: &MgPreconditioner<'_>, level: usize) -> usize {
        if level == 0 {
            mg.fine.n()
        } else {
            let lvl = &mg.coarse[level - 1];
            lvl.nrows * lvl.ncols
        }
    }

    fn level_operator<'x>(mg: &'x MgPreconditioner<'_>, level: usize) -> LevelOperator<'x> {
        mg.level_operator(level)
    }

    // Compare representations using the same smoother. Production explicit
    // MG still selects Gauss–Seidel; only this numerical reference uses Jacobi.
    fn use_jacobi_reference(mg: &MgPreconditioner<'_>) {
        let smoothers: Vec<_> = (0..mg.coarse.len())
            .map(|level| Smoother::weighted_jacobi(&mg.level_operator(level), 0.67))
            .collect();
        let mut workspaces = mg.workspaces.borrow_mut();
        for (workspace, smoother) in workspaces.iter_mut().zip(smoothers) {
            assert!(matches!(workspace.smoother, Smoother::SymmetricGaussSeidel));
            workspace.smoother = smoother;
        }
    }

    fn structural_nnz(op: &dyn Operator) -> usize {
        let mut nnz = 0;
        for row in 0..op.n() {
            op.for_each_entry(row, &mut |_, _| nnz += 1);
        }
        nnz
    }

    fn materialize(op: &dyn Operator) -> CsMat<f64> {
        let mut triplets = sprs::TriMat::new((op.n(), op.n()));
        for row in 0..op.n() {
            op.for_each_entry(row, &mut |col, val| triplets.add_triplet(row, col, val));
        }
        triplets.to_csr()
    }

    fn materialize_transfer(transfer: Transfer) -> CsMat<f64> {
        let mut triplets = sprs::TriMat::new((
            transfer.fine_nrows * transfer.fine_ncols,
            transfer.coarse_n(),
        ));
        for col in 0..transfer.coarse_n() {
            transfer.for_each_column(col, |row, val| triplets.add_triplet(row, col, val));
        }
        triplets.to_csr()
    }

    fn assert_operators_match(actual: &dyn Operator, expected: &dyn Operator) {
        assert_eq!(actual.n(), expected.n());
        for row in 0..actual.n() {
            let mut entries = Vec::new();
            let mut reference = Vec::new();
            actual.for_each_entry(row, &mut |col, val| entries.push((col, val)));
            expected.for_each_entry(row, &mut |col, val| reference.push((col, val)));
            entries.sort_by_key(|&(col, _)| col);
            reference.sort_by_key(|&(col, _)| col);
            assert_eq!(entries.len(), reference.len(), "row {row} structure");
            for ((col, val), (ref_col, ref_val)) in entries.iter().zip(&reference) {
                assert_eq!(col, ref_col, "row {row} column");
                assert!(
                    (val - ref_val).abs() <= 1e-11 * ref_val.abs().max(1.0),
                    "row {row}, col {col}: {val} vs {ref_val}"
                );
            }
        }
        let x: Vec<f64> = (0..actual.n()).map(|i| (i as f64 * 0.3).sin()).collect();
        let mut y = vec![0.0; actual.n()];
        let mut ref_y = y.clone();
        actual.matvec(&x, &mut y);
        expected.matvec(&x, &mut ref_y);
        for (a, b) in y.iter().zip(ref_y) {
            assert!((a - b).abs() <= 1e-10 * b.abs().max(1.0));
        }
        for (a, b) in actual.diag_inv().iter().zip(expected.diag_inv()) {
            assert!((a - b).abs() <= 1e-11 * b.abs().max(1.0));
        }
    }

    fn reference_galerkin(fine: &dyn Operator, p: &CsMat<f64>, coarse_n: usize) -> CsMat<f64> {
        let p_csc = p.to_csc();

        // Symbolic pass: discover distinct columns in each output row before
        // computing values. These counts give exact-sized CSR allocations, avoiding
        // hash maps and the potentially large intermediate product L_fine * P.
        let mut seen = vec![usize::MAX; coarse_n];
        let mut row_nnz = vec![0usize; coarse_n];
        for i in 0..coarse_n {
            let Some(pcol) = p_csc.outer_view(i) else {
                continue;
            };
            for (a, _) in pcol.iter() {
                fine.for_each_entry(a, &mut |b, _| {
                    if let Some(pb) = p.outer_view(b) {
                        for (j, _) in pb.iter() {
                            if seen[j] != i {
                                seen[j] = i;
                                row_nnz[i] += 1;
                            }
                        }
                    }
                });
            }
        }

        let mut indptr = vec![0usize; coarse_n + 1];
        for i in 0..coarse_n {
            indptr[i + 1] = indptr[i] + row_nnz[i];
        }
        let nnz = indptr[coarse_n];

        // Numeric pass: accumulate contributions into the allocated CSR slots.
        seen.fill(usize::MAX);
        let mut pos = vec![0usize; coarse_n];
        let mut row_count = vec![0usize; coarse_n];
        let mut cols = vec![0usize; nnz];
        let mut vals = vec![0.0f64; nnz];
        for i in 0..coarse_n {
            let Some(pcol) = p_csc.outer_view(i) else {
                continue;
            };
            for (a, &p_ai) in pcol.iter() {
                fine.for_each_entry(a, &mut |b, l_ab| {
                    if let Some(pb) = p.outer_view(b) {
                        for (j, &p_bj) in pb.iter() {
                            let slot = if seen[j] != i {
                                seen[j] = i;
                                let s = indptr[i] + row_count[i];
                                row_count[i] += 1;
                                pos[j] = s;
                                cols[s] = j;
                                vals[s] = 0.0;
                                s
                            } else {
                                pos[j]
                            };
                            vals[slot] += p_ai * l_ab * p_bj;
                        }
                    }
                });
            }
        }

        CsMat::new_from_unsorted((coarse_n, coarse_n), indptr, cols, vals).unwrap()
    }

    #[test]
    fn implicit_galerkin_and_level_two_match_original_coarsening() {
        use crate::linalg::operator::{GroundSpec, StencilOperator};
        for (nr, nc) in [(8, 8), (15, 17), (16, 16), (9, 13), (2, 19), (19, 2)] {
            let n = nr * nc;
            let resistance: Vec<f64> = (0..n)
                .map(|i| match i % 13 {
                    0 => -9999.0,
                    1 => 0.0,
                    2 => f64::NAN,
                    3 => 1e-3,
                    4 => 1e6,
                    _ => 1.0 + (i % 9) as f64,
                })
                .collect();
            for ground in 0..3 {
                let spec = match ground {
                    0 => GroundSpec::None,
                    1 => GroundSpec::Neumann(
                        (0..n)
                            .map(|i| if i % nc == nc - 1 { 2.0 } else { 0.0 })
                            .collect(),
                    ),
                    _ => GroundSpec::Dirichlet((0..n).map(|i| i % nc == nc - 1).collect()),
                };
                let fine = StencilOperator::new(nr, nc, -9999.0, &resistance, spec);
                let transfer = Transfer {
                    fine_nrows: nr,
                    fine_ncols: nc,
                    coarse_nrows: nr / 2,
                    coarse_ncols: nc / 2,
                };
                let p = materialize_transfer(transfer);
                let fine_matrix = materialize(&fine);
                let mut reference = reference_galerkin(&fine_matrix, &p, transfer.coarse_n());
                let matvec_workspace = RefCell::new(GalerkinMatvecWorkspace::new(fine.n()));
                let raw = GalerkinOperator {
                    fine: &fine,
                    transfer,
                    reg_diag0: 0.0,
                    matvec_workspace: &matvec_workspace,
                };
                let (reg, nnz) = raw.setup_metadata();
                assert_eq!(nnz, reference.nnz());
                crate::circuit::laplacian::regularize_laplacian(&mut reference);
                let implicit = GalerkinOperator {
                    reg_diag0: reg,
                    ..raw
                };
                assert_operators_match(&implicit, &reference);
                assert_operators_match(
                    &galerkin_coarsen_operator(&fine, transfer),
                    &reference_galerkin(&fine_matrix, &p, transfer.coarse_n()),
                );
                if transfer.coarse_nrows >= 2 && transfer.coarse_ncols >= 2 {
                    let p1 = Transfer {
                        fine_nrows: nr / 2,
                        fine_ncols: nc / 2,
                        coarse_nrows: nr / 4,
                        coarse_ncols: nc / 4,
                    };
                    let mut actual2 = galerkin_coarsen_operator(&implicit, p1);
                    let mut expected2 =
                        reference_galerkin(&reference, &materialize_transfer(p1), p1.coarse_n());
                    crate::circuit::laplacian::regularize_laplacian(&mut actual2);
                    crate::circuit::laplacian::regularize_laplacian(&mut expected2);
                    assert_operators_match(&actual2, &expected2);
                }
            }
        }
    }

    #[test]
    fn galerkin_preserves_structurally_present_zero_entries() {
        let transfer = Transfer {
            fine_nrows: 8,
            fine_ncols: 8,
            coarse_nrows: 4,
            coarse_ncols: 4,
        };
        let mut diagonal = sprs::TriMat::new((64, 64));
        for i in 0..64 {
            diagonal.add_triplet(i, i, 0.0);
        }
        let fine = diagonal.to_csr::<usize>();
        let matvec_workspace = RefCell::new(GalerkinMatvecWorkspace::new(fine.n()));
        let implicit = GalerkinOperator {
            fine: &fine,
            transfer,
            reg_diag0: 0.0,
            matvec_workspace: &matvec_workspace,
        };
        let expected = reference_galerkin(&fine, &materialize_transfer(transfer), 16);
        assert_operators_match(&implicit, &expected);
        assert_eq!(implicit.setup_metadata().1, expected.nnz());
        assert!(expected.nnz() > 16);
    }

    struct ProductOnly<'b> {
        matrix: &'b CsMat<f64>,
        calls: std::cell::Cell<usize>,
    }

    impl Operator for ProductOnly<'_> {
        fn n(&self) -> usize {
            self.matrix.rows()
        }
        fn matvec(&self, x: &[f64], y: &mut [f64]) {
            self.calls.set(self.calls.get() + 1);
            self.matrix.matvec(x, y);
        }
        fn diag_inv(&self) -> Vec<f64> {
            panic!("diagonal must be cached before smoothing")
        }
        fn for_each_entry(&self, _: usize, _: &mut dyn FnMut(usize, f64)) {
            panic!("matvec-based smoothing must not reconstruct coefficients")
        }
    }

    #[test]
    fn weighted_jacobi_updates_simultaneously_using_only_matvec() {
        let mut entries = sprs::TriMat::new((3, 3));
        for i in 0..3 {
            entries.add_triplet(i, i, 2.0);
        }
        for i in 0..2 {
            entries.add_triplet(i, i + 1, -1.0);
            entries.add_triplet(i + 1, i, -1.0);
        }
        let matrix = entries.to_csr::<usize>();
        let op = ProductOnly {
            matrix: &matrix,
            calls: std::cell::Cell::new(0),
        };
        let mut e = vec![1.0, 2.0, 4.0];
        let mut scratch = vec![99.0; 3];
        let ptr = scratch.as_ptr();
        weighted_jacobi_smooth(&op, &mut e, &[3.0, -2.0, 1.0], &[0.5; 3], 0.5, &mut scratch);
        assert_eq!(e, vec![1.75, 1.75, 2.75]);
        weighted_jacobi_smooth(&op, &mut e, &[3.0, -2.0, 1.0], &[0.5; 3], 0.5, &mut scratch);
        assert_eq!(e, vec![2.0625, 1.5, 2.0625]);
        assert_eq!(op.calls.get(), 2);
        assert_eq!(scratch.as_ptr(), ptr);
    }

    #[test]
    fn jacobi_damping_is_capped_for_large_normalized_eigenvalues() {
        let mut entries = sprs::TriMat::new((4, 4));
        for i in 0..4 {
            for j in 0..4 {
                entries.add_triplet(i, j, if i == j { 1.0 } else { 0.9 });
            }
        }
        let matrix = entries.to_csr::<usize>();
        let smoother = Smoother::weighted_jacobi(&matrix, 0.67);
        let Smoother::WeightedJacobi { diag_inv, omega } = &smoother else {
            panic!("expected Jacobi")
        };
        assert_eq!(diag_inv, &vec![1.0; 4]);
        assert!((*omega - 1.8 / 3.7).abs() < 1e-14);
        let mut e = vec![1.0; 4];
        smoother.smooth(&matrix, &mut e, &[0.0; 4], &mut [0.0; 4], 0.67);
        for value in e {
            assert!((value + 0.8).abs() < 1e-14);
        }
    }

    #[test]
    fn composed_galerkin_applies_fine_operator_once_and_reuses_scratch() {
        let nr = 8;
        let nc = 8;
        let resistance = vec![1.0; nr * nc];
        let (_, _, _, matrix) = crate::build_circuit_model(&resistance, nr, nc, -9999.0);
        let transfer = Transfer {
            fine_nrows: nr,
            fine_ncols: nc,
            coarse_nrows: 4,
            coarse_ncols: 4,
        };
        let mut expected = reference_galerkin(&matrix, &materialize_transfer(transfer), 16);
        let reg = 0.123;
        expected.get_mut(0, 0).map(|diag| *diag += reg).unwrap();
        let fine = ProductOnly {
            matrix: &matrix,
            calls: std::cell::Cell::new(0),
        };
        let scratch = RefCell::new(GalerkinMatvecWorkspace::new(64));
        let ptrs = {
            let w = scratch.borrow();
            (w.prolonged.as_ptr(), w.applied.as_ptr())
        };
        let op = GalerkinOperator {
            fine: &fine,
            transfer,
            reg_diag0: reg,
            matvec_workspace: &scratch,
        };
        let x: Vec<f64> = (0..16).map(|i| (i as f64 * 0.3).sin()).collect();
        let mut actual = vec![77.0; 16];
        let mut reference = vec![0.0; 16];
        expected.matvec(&x, &mut reference);
        for _ in 0..2 {
            op.matvec(&x, &mut actual);
            for (a, b) in actual.iter().zip(&reference) {
                assert!((a - b).abs() < 1e-12);
            }
        }
        op.matvec(&[0.0; 16], &mut actual);
        assert_eq!(actual, vec![0.0; 16]);
        assert_eq!(fine.calls.get(), 3);
        let w = scratch.borrow();
        assert_eq!((w.prolonged.as_ptr(), w.applied.as_ptr()), ptrs);
    }

    #[test]
    fn small_grids_and_cholesky_size_reuse_workspaces() {
        use crate::linalg::operator::{GroundSpec, StencilOperator};
        for (nr, nc, depth, target, expected) in [
            (1, 1, 8, None, 1),
            (3, 5, 8, None, 15),
            (7, 7, 8, None, 49),
            (16, 16, 1, None, 256),
            (3, 64, 8, None, 192),
            (16, 16, 8, Some(64), 64),
            (16, 16, 8, Some(4), 4),
            (16, 16, 8, Some(1), 1),
            (16, 16, 2, Some(1), 64),
        ] {
            let resistance = vec![1.0; nr * nc];
            let (_, _, _, laplacian) = crate::build_circuit_model(&resistance, nr, nc, -9999.0);
            let laplacian =
                crate::circuit::laplacian::add_diagonal(&laplacian, &vec![1.0; nr * nc]);
            let options = MgOptions {
                max_levels: depth,
                cholesky_size: target,
            };
            memory::reset();
            let explicit = MgPreconditioner::build_with_options(
                FineOperator::Explicit(laplacian),
                nr,
                nc,
                options,
            );
            #[cfg(feature = "instrumentation-profile")]
            {
                let profile = memory::take_profile().unwrap();
                assert!(profile.hierarchy[0].laplacian_bytes > 0);
                for level in &profile.hierarchy {
                    let terminal = level.level == profile.hierarchy.len() - 1;
                    assert_eq!(
                        level.smoother_repr,
                        if terminal {
                            "none"
                        } else {
                            "symmetric-gauss-seidel"
                        }
                    );
                    assert_eq!(level.smoother_diag_bytes, 0);
                    assert_eq!(level.operator_matvec_scratch_bytes, 0);
                }
                for level in &profile.hierarchy[1..] {
                    // CSR output arrays are retained in the Laplacian, not scratch.
                    assert_eq!(
                        level.laplacian_bytes,
                        memory::vec_usize_bytes(level.nodes + 1)
                            + memory::vec_usize_bytes(level.nnz)
                            + memory::vec_f64_bytes(level.nnz)
                    );
                    assert_eq!(level.prolongation_p_bytes, 0);
                    assert_eq!(level.galerkin_scratch.p_triplets_clone_bytes, 0);
                    assert_eq!(level.galerkin_scratch.p_csr_bytes, 0);
                    assert_eq!(level.galerkin_scratch.p_csc_bytes, 0);
                }
            }
            memory::reset();
            let stencil = StencilOperator::new(
                nr,
                nc,
                -9999.0,
                &resistance,
                GroundSpec::Neumann(vec![1.0; nr * nc]),
            );
            let stencil = MgPreconditioner::build_with_options(
                FineOperator::Stencil(stencil),
                nr,
                nc,
                options,
            );
            assert_eq!(level_n(&explicit, explicit.coarse.len()), expected);
            assert_eq!(level_n(&stencil, stencil.coarse.len()), expected);
            #[cfg(feature = "instrumentation-profile")]
            {
                let profile = memory::take_profile().unwrap();
                assert_eq!(profile.coarsest_nodes, expected);
                assert_eq!(profile.requested_cholesky_size, target);
                assert_eq!(profile.hierarchy[0].laplacian_bytes, 0);
                assert_eq!(profile.ground_storage_bytes, memory::vec_f64_bytes(nr * nc));
                assert_eq!(profile.hierarchy[0].operator_repr, "stencil");
                for level in &profile.hierarchy {
                    let terminal = level.level == profile.hierarchy.len() - 1;
                    assert_eq!(
                        level.smoother_repr,
                        if terminal { "none" } else { "weighted-jacobi" }
                    );
                    assert_eq!(
                        level.smoother_diag_bytes,
                        if terminal {
                            0
                        } else {
                            memory::vec_f64_bytes(level.nodes)
                        }
                    );
                    assert_eq!(
                        level.operator_matvec_scratch_bytes,
                        if level.level == 1 {
                            memory::vec_f64_bytes(2 * nr * nc)
                        } else {
                            0
                        }
                    );
                }
                for level in &profile.hierarchy[1..] {
                    assert_eq!(level.prolongation_p_bytes, 0);
                    assert_eq!(level.galerkin_scratch.p_triplets_clone_bytes, 0);
                    assert_eq!(level.galerkin_scratch.p_csr_bytes, 0);
                    assert_eq!(level.galerkin_scratch.p_csc_bytes, 0);
                    if level.level == 1 {
                        assert_eq!(level.operator_repr, "galerkin-stencil");
                        assert_eq!(level.laplacian_bytes, 0);
                        assert_eq!(level.galerkin_scratch.symbolic_seen_bytes, 0);
                        assert_eq!(level.galerkin_scratch.numeric_pos_bytes, 0);
                        assert_eq!(
                            level.galerkin_scratch.row_accumulator_bytes,
                            std::mem::size_of::<GalerkinRow>() as u64
                        );
                    } else {
                        assert_eq!(level.operator_repr, "explicit");
                        assert!(level.laplacian_bytes > 0);
                    }
                }
            }
            let source: Vec<f64> = (0..nr * nc).map(|i| 1.0 + (i as f64).sin()).collect();
            let (mut first, mut again, mut reference) = (Vec::new(), Vec::new(), Vec::new());
            stencil.apply(&source, &mut first);
            stencil.apply(&vec![0.0; nr * nc], &mut again);
            stencil.apply(&source, &mut again);
            use_jacobi_reference(&explicit);
            explicit.apply(&source, &mut reference);
            assert_eq!(
                first, again,
                "scratch reuse must not retain previous solutions"
            );
            for (&a, &b) in first.iter().zip(&reference) {
                assert!((a - b).abs() < 1e-8);
            }
        }
    }

    #[test]
    fn test_stencil_matches_explicit_mg() {
        use crate::linalg::operator::{GroundSpec, StencilOperator};

        let nrows = 16;
        let ncols = 16;
        let n = nrows * ncols;
        let nodata = crate::NODATA_SENTINEL;

        let mut resistance = vec![1.0; n];
        for r in 0..nrows {
            for c in 0..ncols {
                let i = r * ncols + c;
                if c % 4 == 0 {
                    resistance[i] = 0.001;
                }
                if r % 5 == 3 {
                    resistance[i] = 1000.0;
                }
            }
        }
        resistance[5 * ncols + 5] = nodata;

        let mut gnd = vec![0.0; n];
        gnd[n - 1] = 1.0;
        gnd[n - ncols] = 2.0;

        let filled = crate::raster::fill_nodata(&resistance, nodata);
        let (_ctn, _nn, _e, lap) = crate::build_circuit_model(&filled, nrows, ncols, nodata);

        let shunt: Vec<f64> = gnd.iter().map(|&g| if g > 0.0 { g } else { 0.0 }).collect();
        let mask: Vec<bool> = gnd.iter().map(|&g| g > 0.0).collect();
        let gns: Vec<usize> = (0..n).filter(|&i| mask[i]).collect();

        let x: Vec<f64> = (0..n).map(|i| (i as f64 * 0.13).sin()).collect();

        let check = |mg_e: &MgPreconditioner, mg_s: &MgPreconditioner, tag: &str| {
            assert_eq!(
                structural_nnz(&mg_e.level_operator(1)),
                structural_nnz(&mg_s.level_operator(1)),
                "{tag}: coarse level-1 nnz differs"
            );
            use_jacobi_reference(mg_e);
            let mut ze = vec![0.0; n];
            let mut zs = vec![0.0; n];
            mg_e.apply(&x, &mut ze);
            mg_s.apply(&x, &mut zs);
            for i in 0..n {
                assert!(
                    (ze[i] - zs[i]).abs() < 1e-6,
                    "{tag}: MG stencil vs explicit mismatch at {}: {} vs {}",
                    i,
                    ze[i],
                    zs[i]
                );
            }
        };

        // No ground
        {
            let mg_e = MgPreconditioner::build_from_laplacian(lap.clone(), nrows, ncols, 5);
            let st = StencilOperator::new(nrows, ncols, nodata, &resistance, GroundSpec::None);
            let mg_s =
                MgPreconditioner::build_from_operator(FineOperator::Stencil(st), nrows, ncols, 5);
            check(&mg_e, &mg_s, "none");
        }

        // Neumann ground
        {
            let a = crate::circuit::laplacian::add_diagonal(&lap, &shunt);
            let mg_e = MgPreconditioner::build_from_laplacian(a, nrows, ncols, 5);
            let st = StencilOperator::new(
                nrows,
                ncols,
                nodata,
                &resistance,
                GroundSpec::Neumann(shunt.clone()),
            );
            let mg_s =
                MgPreconditioner::build_from_operator(FineOperator::Stencil(st), nrows, ncols, 5);
            check(&mg_e, &mg_s, "neumann");
        }

        // Dirichlet ground
        {
            let a = crate::solve::apply_dirichlet_ground_lap(&lap, &gns);
            let mg_e = MgPreconditioner::build_from_laplacian(a, nrows, ncols, 5);
            let st = StencilOperator::new(
                nrows,
                ncols,
                nodata,
                &resistance,
                GroundSpec::Dirichlet(mask.clone()),
            );
            let mg_s =
                MgPreconditioner::build_from_operator(FineOperator::Stencil(st), nrows, ncols, 5);
            check(&mg_e, &mg_s, "dirichlet");
        }
    }

    #[test]
    fn matrix_free_preconditioner_is_symmetric_and_positive() {
        use crate::linalg::operator::{GroundSpec, StencilOperator};
        for (nr, nc) in [(16, 16), (17, 19)] {
            let n = nr * nc;
            let resistance: Vec<f64> = (0..n).map(|i| 0.5 + (i % 7) as f64).collect();
            let x: Vec<f64> = (0..n).map(|i| (i as f64 * 0.1).sin()).collect();
            let y: Vec<f64> = (0..n).map(|i| (i as f64 * 0.2).cos()).collect();
            for mode in 0..3 {
                let spec = match mode {
                    0 => GroundSpec::None,
                    1 => GroundSpec::Neumann(
                        (0..n)
                            .map(|i| if i % nc == nc - 1 { 1.0 } else { 0.0 })
                            .collect(),
                    ),
                    _ => GroundSpec::Dirichlet((0..n).map(|i| i % nc == nc - 1).collect()),
                };
                let fine = StencilOperator::new(nr, nc, -9999.0, &resistance, spec);
                let mg =
                    MgPreconditioner::build_from_operator(FineOperator::Stencil(fine), nr, nc, 8);
                let mut mx = Vec::new();
                let mut my = Vec::new();
                mg.apply(&x, &mut mx);
                mg.apply(&y, &mut my);
                let x_my: f64 = x.iter().zip(&my).map(|(a, b)| a * b).sum();
                let y_mx: f64 = y.iter().zip(&mx).map(|(a, b)| a * b).sum();
                assert!((x_my - y_mx).abs() < 1e-9 * x_my.abs().max(y_mx.abs()).max(1.0));
                assert!(x.iter().zip(&mx).map(|(a, b)| a * b).sum::<f64>() > 0.0);
                for level in 0..num_levels(&mg) {
                    let op = mg.level_operator(level);
                    for diagonal in op.diag_inv() {
                        assert!(diagonal > 0.0);
                    }
                    let mut error: Vec<f64> = (0..op.n())
                        .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
                        .collect();
                    let before: f64 = error.iter().map(|x| x * x).sum();
                    for _ in 0..3 {
                        mg.workspaces.borrow()[level].smoother.smooth(
                            &op,
                            &mut error,
                            &vec![0.0; op.n()],
                            &mut vec![0.0; op.n()],
                            mg.omega,
                        );
                    }
                    assert!(error.iter().map(|x| x * x).sum::<f64>() < before);
                }
            }
        }
    }

    #[test]
    fn test_multigrid_preconditioner_symmetry() {
        let nrows = 15;
        let ncols = 15;
        let n = nrows * ncols;

        // 1. Build using your actual `.build` method
        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = build_mg(&resistance, nrows, ncols, 3);

        // 2. Generate two distinct structural vectors using predictable trigonometric waves
        let x: Vec<f64> = (0..n).map(|i| (i as f64 * 0.1).sin()).collect();
        let y: Vec<f64> = (0..n).map(|i| (i as f64 * 0.2).cos()).collect();

        let mut mx = vec![0.0; n];
        let mut my = vec![0.0; n];

        let mut ax = vec![0.0; n];
        let mut ay = vec![0.0; n];
        pcg::mat_vec_mul_into(fine_csmat(&mg), &x, &mut ax);
        pcg::mat_vec_mul_into(fine_csmat(&mg), &y, &mut ay);
        let dot_x_ay: f64 = x.iter().zip(ay.iter()).map(|(a, b)| a * b).sum();
        let dot_y_ax: f64 = y.iter().zip(ax.iter()).map(|(a, b)| a * b).sum();
        assert!(
            (dot_x_ay - dot_y_ax).abs() < 1e-7,
            "Fine matrix A is asymmetric!"
        );

        mg.apply(&x, &mut mx);
        mg.apply(&y, &mut my);

        // 3. Compute dot products: x · M(y) vs y · M(x)
        let dot_x_my: f64 = x.iter().zip(my.iter()).map(|(a, b)| a * b).sum();
        let dot_y_mx: f64 = y.iter().zip(mx.iter()).map(|(a, b)| a * b).sum();

        // Operators must be symmetric up to floating-point drift
        assert!(
            (dot_x_my - dot_y_mx).abs() < 1e-6,
            "MG Preconditioner is asymmetric! x*M(y) = {}, y*M(x) = {}",
            dot_x_my,
            dot_y_mx
        );
    }

    #[test]
    fn test_mg_preconditioned_cg_converges() {
        let nrows = 64;
        let ncols = 64;
        let n = nrows * ncols;

        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = build_mg(&resistance, nrows, ncols, 7);

        let b_raw: Vec<f64> = (0..n)
            .map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5)
            .collect();
        let mean: f64 = b_raw.iter().sum::<f64>() / n as f64;
        let b: Vec<f64> = b_raw.iter().map(|v| v - mean).collect();

        let mut x = vec![0.0; n];
        let mut r = b.clone();
        let b_norm = b.iter().map(|v| v * v).sum::<f64>().sqrt();

        let mut residuals = Vec::new();

        for _iter in 0..50 {
            let r_norm = r.iter().map(|v| v * v).sum::<f64>().sqrt();
            residuals.push(r_norm);

            let mut z = vec![0.0; n];
            mg.apply(&r, &mut z);

            let mut az = vec![0.0; n];
            pcg::mat_vec_mul_into(fine_csmat(&mg), &z, &mut az);
            let z_az: f64 = z.iter().zip(az.iter()).map(|(a, b)| a * b).sum();
            if z_az.abs() < 1e-30 {
                break;
            }

            let alpha = {
                let rz: f64 = r.iter().zip(z.iter()).map(|(a, b)| a * b).sum();
                rz / z_az
            };

            for i in 0..n {
                x[i] += alpha * z[i];
                r[i] -= alpha * az[i];
            }

            if r_norm / b_norm < 1e-6 {
                break;
            }
        }

        let r0 = residuals[0];
        let r_last = residuals[residuals.len() - 1];
        eprintln!(
            "Large uniform {}x{}: {} iters, ||r|| went from {:.6e} to {:.6e} (ratio {:.4e})",
            nrows,
            ncols,
            residuals.len(),
            r0,
            r_last,
            r_last / r0
        );
        assert!(
            r_last < r0 * 1e-2,
            "MG-preconditioned CG did not converge on large uniform grid: ||r|| went from {:.6e} to {:.6e} in {} iters",
            r0, r_last, residuals.len()
        );
    }

    #[test]
    fn test_laplacian_diagonal_positive() {
        let nrows = 15;
        let ncols = 15;
        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = build_mg(&resistance, nrows, ncols, 4);

        for level in 0..num_levels(&mg) {
            let n = level_n(&mg, level);
            let op = level_operator(&mg, level);
            for row in 0..n {
                op.for_each_entry(row, &mut |col, val| {
                    if col == row {
                        assert!(
                            val > 0.0,
                            "Level {}: diagonal[{}] = {:.6e} (must be positive)",
                            level,
                            row,
                            val
                        );
                        let diag_inv = if val.abs() > 1e-15 {
                            1.0 / val.abs()
                        } else {
                            0.0
                        };
                        assert!(
                            diag_inv > 0.0,
                            "Level {}: diag_inv[{}] = {:.6e} (must be positive)",
                            level,
                            row,
                            diag_inv
                        );
                    }
                });
            }
        }
    }

    #[test]
    fn test_preconditioner_positive_definite() {
        let nrows = 15;
        let ncols = 15;
        let n = nrows * ncols;
        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = build_mg(&resistance, nrows, ncols, 4);

        let z: Vec<f64> = (0..n)
            .map(|i| (i as f64 * 0.7 + 1.3).sin() * 0.5 + 0.3)
            .collect();
        let mut mz = vec![0.0; n];
        mg.apply(&z, &mut mz);

        let z_mz: f64 = z.iter().zip(mz.iter()).map(|(a, b)| a * b).sum();
        assert!(
            z_mz > 0.0,
            "Preconditioner is not positive definite: zᵀ·M⁻¹·z = {:.6e}",
            z_mz
        );

        let z2: Vec<f64> = (0..n)
            .map(|i| (i as f64 * 1.1 + 2.7).cos() * 0.8 - 0.2)
            .collect();
        let mut mz2 = vec![0.0; n];
        mg.apply(&z2, &mut mz2);
        let z2_mz2: f64 = z2.iter().zip(mz2.iter()).map(|(a, b)| a * b).sum();
        assert!(
            z2_mz2 > 0.0,
            "Preconditioner is not positive definite (vector 2): zᵀ·M⁻¹·z = {:.6e}",
            z2_mz2
        );
    }

    #[test]
    fn test_preconditioned_search_direction_aligns_with_residual() {
        let nrows = 15;
        let ncols = 15;
        let n = nrows * ncols;
        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = build_mg(&resistance, nrows, ncols, 4);

        let b_raw: Vec<f64> = (0..n)
            .map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5)
            .collect();
        let mean: f64 = b_raw.iter().sum::<f64>() / n as f64;
        let b: Vec<f64> = b_raw.iter().map(|v| v - mean).collect();

        let mut z = vec![0.0; n];
        mg.apply(&b, &mut z);

        let r_z: f64 = b.iter().zip(z.iter()).map(|(a, b)| a * b).sum();
        assert!(
            r_z > 0.0,
            "Preconditioned residual is anti-aligned with residual: rᵀ·z = {:.6e} (must be > 0)",
            r_z
        );
    }

    #[test]
    fn test_coarse_solve_correctness() {
        let nrows = 15;
        let ncols = 15;
        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = build_mg(&resistance, nrows, ncols, 4);

        let lvl = mg.coarse.last().unwrap();
        let n = lvl.nrows * lvl.ncols;

        assert!(
            lvl.cholesky_l.is_some(),
            "Coarsest level should have Cholesky factorization"
        );

        let b: Vec<f64> = (0..n)
            .map(|i| ((i as f64 * 0.5 + 0.3).sin() + 1.0) * 0.5)
            .collect();
        let l = lvl.cholesky_l.as_ref().unwrap();
        let z = crate::linalg::cholesky::cholesky_solve(l, &b, n);

        let mut az = vec![0.0; n];
        lvl.operator(&mg.fine).matvec(&z, &mut az);

        let mut max_err = 0.0;
        for i in 0..n {
            let err = (az[i] - b[i]).abs();
            if err > max_err {
                max_err = err;
            }
        }
        assert!(
            max_err < 1e-4,
            "Coarsest level solve error too large: max|Az - b| = {:.6e}",
            max_err
        );
    }

    #[test]
    fn test_smoother_converges_on_all_levels() {
        let nrows = 15;
        let ncols = 15;
        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = build_mg(&resistance, nrows, ncols, 4);

        for level in 0..num_levels(&mg) {
            let n = level_n(&mg, level);
            let b = vec![0.0; n];
            let mut x: Vec<f64> = (0..n)
                .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
                .collect();

            let initial_norm = x.iter().map(|v| v * v).sum::<f64>().sqrt();
            for _ in 0..3 {
                symmetric_gauss_seidel_smooth(&level_operator(&mg, level), &mut x, &b, mg.omega);
            }
            let final_norm = x.iter().map(|v| v * v).sum::<f64>().sqrt();

            assert!(
                final_norm < initial_norm,
                "Level {}: smoother did not reduce error norm (initial={:.6e}, final={:.6e})",
                level,
                initial_norm,
                final_norm
            );
        }
    }

    #[test]
    fn test_mg_preconditioner_on_full_grid() {
        let nrows = 8;
        let ncols = 8;
        let n = nrows * ncols;

        let resistance = generate_mock_resistance(nrows, ncols);
        let (_cell_to_node, num_nodes, _edges, _full_lap) =
            crate::build_circuit_model(&resistance, nrows, ncols, -1.0);
        assert_eq!(num_nodes, n);

        let mg = build_mg(&resistance, nrows, ncols, 4);

        let b_raw: Vec<f64> = (0..n)
            .map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5)
            .collect();
        let mean: f64 = b_raw.iter().sum::<f64>() / n as f64;
        let b: Vec<f64> = b_raw.iter().map(|v| v - mean).collect();

        let mut z = vec![0.0; n];
        mg.apply(&b, &mut z);

        let mut az = vec![0.0; n];
        pcg::mat_vec_mul_into(fine_csmat(&mg), &z, &mut az);
        let z_az: f64 = z.iter().zip(az.iter()).map(|(a, b)| a * b).sum();
        let rz: f64 = b.iter().zip(z.iter()).map(|(a, b)| a * b).sum();

        let b_norm = b.iter().map(|v| v * v).sum::<f64>().sqrt();
        eprintln!("Full-grid test: n={}", n);
        eprintln!(
            "  ||b|| = {:.6e}, r·z = {:.6e}, z·Az = {:.6e}",
            b_norm, rz, z_az
        );

        assert!(
            z_az > 0.0,
            "MG preconditioner on full grid: z·Az = {:.6e} (must be > 0)",
            z_az
        );
        assert!(
            rz > 0.0,
            "MG preconditioner on full grid: r·z = {:.6e} (must be > 0)",
            rz
        );
    }

    #[test]
    fn test_diag_inv_range_across_levels() {
        let nrows = 32;
        let ncols = 32;
        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = build_mg(&resistance, nrows, ncols, 6);

        for level in 0..num_levels(&mg) {
            let op = level_operator(&mg, level);
            let diag_inv = op.diag_inv();
            let mut min_inv = f64::MAX;
            let mut max_inv = 0.0;
            for &d in &diag_inv {
                if d < min_inv {
                    min_inv = d;
                }
                if d > max_inv {
                    max_inv = d;
                }
            }
            eprintln!(
                "Level {}: diag_inv range = [{:.6e}, {:.6e}] (ratio = {:.2e})",
                level,
                min_inv,
                max_inv,
                max_inv / min_inv.max(1e-30)
            );

            assert!(
                min_inv > 1e-15,
                "Level {}: diag_inv too small: min = {:.6e}",
                level,
                min_inv
            );
            assert!(
                max_inv < 1e10,
                "Level {}: diag_inv too large: max = {:.6e}",
                level,
                max_inv
            );
        }
    }

    #[test]
    fn test_coarse_cholesky_accuracy() {
        let nrows = 32;
        let ncols = 32;
        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = build_mg(&resistance, nrows, ncols, 6);

        let lvl = mg.coarse.last().unwrap();
        let n = lvl.nrows * lvl.ncols;

        assert!(
            lvl.cholesky_l.is_some(),
            "Coarsest level should have Cholesky factorization"
        );

        let l = lvl.cholesky_l.as_ref().unwrap();
        let b: Vec<f64> = (0..n)
            .map(|i| ((i as f64 * 0.5 + 0.3).sin() + 1.0) * 0.5)
            .collect();
        let x = crate::linalg::cholesky::cholesky_solve(l, &b, n);

        let mut ax = vec![0.0; n];
        lvl.operator(&mg.fine).matvec(&x, &mut ax);

        let mut max_err = 0.0;
        for i in 0..n {
            let err = (ax[i] - b[i]).abs();
            if err > max_err {
                max_err = err;
            }
        }
        assert!(
            max_err < 1e-6,
            "Coarsest level Cholesky solve error: max|Ax - b| = {:.6e}",
            max_err
        );
    }

    #[test]
    fn test_mg_preconditioned_cg_variable_coefficients() {
        let nrows = 32;
        let ncols = 32;
        let n = nrows * ncols;

        let mut resistance = vec![1.0; n];
        for r in 0..nrows {
            for c in 0..ncols {
                let idx = r * ncols + c;
                if c % 8 == 0 {
                    resistance[idx] = 0.001;
                }
                if r % 8 == 4 {
                    resistance[idx] = 1e6;
                }
            }
        }

        let mg = build_mg(&resistance, nrows, ncols, 6);

        for level in 0..num_levels(&mg) {
            let diag_inv = level_operator(&mg, level).diag_inv();
            let mut min_d = f64::MAX;
            let mut max_d = 0.0;
            for &d in &diag_inv {
                if d < min_d {
                    min_d = d;
                }
                if d > max_d {
                    max_d = d;
                }
            }
            eprintln!(
                "Level {}: diag_inv [{:.4e}, {:.4e}] ratio {:.2e}",
                level,
                min_d,
                max_d,
                max_d / min_d.max(1e-30)
            );
        }

        let b_raw: Vec<f64> = (0..n)
            .map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5)
            .collect();
        let mean: f64 = b_raw.iter().sum::<f64>() / n as f64;
        let b: Vec<f64> = b_raw.iter().map(|v| v - mean).collect();

        let mut x = vec![0.0; n];
        let mut r = b.clone();
        let b_norm = b.iter().map(|v| v * v).sum::<f64>().sqrt();

        let mut residuals = Vec::new();

        for _iter in 0..20 {
            let r_norm = r.iter().map(|v| v * v).sum::<f64>().sqrt();
            residuals.push(r_norm);

            let mut z = vec![0.0; n];
            mg.apply(&r, &mut z);

            let mut az = vec![0.0; n];
            pcg::mat_vec_mul_into(fine_csmat(&mg), &z, &mut az);
            let z_az: f64 = z.iter().zip(az.iter()).map(|(a, b)| a * b).sum();
            let rz: f64 = r.iter().zip(z.iter()).map(|(a, b)| a * b).sum();
            eprintln!(
                "  iter {}: ||r||={:.4e} r·z={:.4e} z·Az={:.4e}",
                _iter, r_norm, rz, z_az
            );

            if z_az.abs() < 1e-30 {
                break;
            }

            let alpha = rz / z_az;

            for i in 0..n {
                x[i] += alpha * z[i];
                r[i] -= alpha * az[i];
            }

            if r_norm / b_norm < 1e-6 {
                break;
            }
        }

        let r0 = residuals[0];
        let r_last = residuals[residuals.len() - 1];
        eprintln!(
            "Variable coeff {}x{}: {} iters, ||r|| went from {:.6e} to {:.6e} (ratio {:.4e})",
            nrows,
            ncols,
            residuals.len(),
            r0,
            r_last,
            r_last / r0
        );
        assert!(
            r_last < r0,
            "MG-preconditioned CG diverged on variable coefficient grid: ||r|| went from {:.6e} to {:.6e}",
            r0, r_last
        );
    }

    #[test]
    fn test_smoother_only_preconditioner() {
        let nrows = 32;
        let ncols = 32;
        let n = nrows * ncols;

        let mut resistance = vec![1.0; n];
        for r in 0..nrows {
            for c in 0..ncols {
                let idx = r * ncols + c;
                if c % 8 == 0 {
                    resistance[idx] = 0.001;
                }
                if r % 8 == 4 {
                    resistance[idx] = 1e6;
                }
            }
        }

        let mg = build_mg(&resistance, nrows, ncols, 6);

        let b_raw: Vec<f64> = (0..n)
            .map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5)
            .collect();
        let mean: f64 = b_raw.iter().sum::<f64>() / n as f64;
        let b: Vec<f64> = b_raw.iter().map(|v| v - mean).collect();

        let mut x = vec![0.0; n];
        let mut r = b.clone();
        let b_norm = b.iter().map(|v| v * v).sum::<f64>().sqrt();

        let mut residuals = Vec::new();

        for _iter in 0..50 {
            let r_norm = r.iter().map(|v| v * v).sum::<f64>().sqrt();
            residuals.push(r_norm);

            let mut z = vec![0.0; n];
            mg.apply(&r, &mut z);

            let mut az = vec![0.0; n];
            pcg::mat_vec_mul_into(fine_csmat(&mg), &z, &mut az);
            let z_az: f64 = z.iter().zip(az.iter()).map(|(a, b)| a * b).sum();
            let rz: f64 = r.iter().zip(z.iter()).map(|(a, b)| a * b).sum();

            if z_az.abs() < 1e-30 {
                break;
            }

            let alpha = rz / z_az;

            for i in 0..n {
                x[i] += alpha * z[i];
                r[i] -= alpha * az[i];
            }

            if r_norm / b_norm < 1e-6 {
                break;
            }
        }

        let r0 = residuals[0];
        let r_last = residuals[residuals.len() - 1];
        eprintln!(
            "Smoother-only {}x{}: {} iters, ||r|| went from {:.6e} to {:.6e} (ratio {:.4e})",
            nrows,
            ncols,
            residuals.len(),
            r0,
            r_last,
            r_last / r0
        );
    }
}
