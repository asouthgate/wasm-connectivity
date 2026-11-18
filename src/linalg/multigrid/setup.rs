use super::{
    CoarseOperator, LevelOperator, LevelWorkspace, MgLevel, MgOptions, MgPreconditioner,
};
use super::galerkin::{galerkin_coarsen_operator, GalerkinMatvecWorkspace, GalerkinOperator};
#[cfg(feature = "instrumentation-profile")]
use super::galerkin::GalerkinRow;
use crate::linalg::{cholesky, operator::{FineOperator, Operator}, real::{scalar, Real}, smooth::Smoother, timing, transfer::Transfer};
use crate::memory;
#[cfg(feature = "instrumentation-profile")]
use crate::memory::GalerkinScratch;
use sprs::CsMat;
use std::cell::RefCell;

// 7×7 is the largest square fine grid below the default 8×8 coarsening threshold.
// Bound dense allocation when a thin or depth-limited grid has no coarse level.
const SMALL_FINE_DIRECT_LIMIT: usize = 49;


#[cfg(feature = "instrumentation-profile")]
fn record_level_memory<F: Real>(coarse: &[MgLevel<F>], fine_cholesky_l: Option<&[F]>) {
    memory::record_fine_level_typed::<F>(
        fine_cholesky_l
            .map(|factor| memory::vec_scalar_bytes::<F>(factor.len()))
            .unwrap_or(0),
    );
    for (k, lvl) in coarse.iter().enumerate() {
        let nodes = lvl.nrows * lvl.ncols;
        let (lap_bytes, repr, scratch) = match &lvl.laplacian {
            CoarseOperator::Galerkin { .. } => (
                0,
                "galerkin-stencil",
                GalerkinScratch {
                    row_accumulator_bytes: std::mem::size_of::<GalerkinRow<F>>() as u64,
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
                        std::mem::size_of::<GalerkinRow<F>>() as u64
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
            .map(|l| memory::vec_scalar_bytes::<F>(l.len()))
            .unwrap_or(0);
        memory::record_level_typed::<F>(
            k + 1,
            nodes,
            lvl.nnz(),
            lap_bytes,
            0,
            cholesky_bytes,
            scratch,
        );
        memory::set_level_operator_representation(k + 1, repr);
    }
}

fn build_level_workspaces<F: Real>(
    fine: &FineOperator<'_, F>,
    coarse: &[MgLevel<F>],
    options: MgOptions,
) -> Vec<LevelWorkspace<F>> {
    // Cache smoother data during setup; no allocations during a sweep.
    let mut workspaces = Vec::with_capacity(1 + coarse.len());
    for level in 0..=coarse.len() {
        let op = if level == 0 {
            LevelOperator::Borrowed(fine)
        } else {
            coarse[level - 1].operator(fine)
        };
        let n = op.n();
        let omega_override = if level == 0 { options.l0omega } else { options.coarseomega }.map(scalar::<F>);
        let mut smoother = if level < coarse.len() {
            // Without an override, the spectral bound selects each level's weight.
            Smoother::weighted_jacobi(&op, scalar::<F>(1.0))
        } else {
            Smoother::SymmetricGaussSeidel
        };
        if let (Some(value), Smoother::WeightedJacobi { omega, .. }) = (omega_override, &mut smoother) {
            *omega = value;
        }
        #[cfg(feature = "instrumentation-profile")]
        {
            let (repr, diag_bytes, omega) = if level == coarse.len() {
                ("none", 0, None)
            } else {
                match &smoother {
                    Smoother::SymmetricGaussSeidel => ("symmetric-gauss-seidel", 0, Some(omega_override.unwrap_or(scalar::<F>(0.67)))),
                    Smoother::WeightedJacobi { diag_inv, omega } => (
                        "weighted-jacobi",
                        memory::vec_scalar_bytes::<F>(diag_inv.len()),
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
                        memory::vec_scalar_bytes::<F>(scratch.prolonged.len() + scratch.applied.len())
                    }
                    CoarseOperator::Explicit(_) => 0,
                }
            } else {
                0
            };
            memory::set_level_smoother_and_operator_scratch(
                level,
                repr,
                diag_bytes,
                omega.map(|v| v.to_f64().unwrap()),
                matvec_bytes,
            );
        }
        let nu = if level == coarse.len() {
            0
        } else if level >= 2 {
            options.coarsenu
        } else if level == 1 {
            options.l1nu.unwrap_or(options.l0nu)
        } else {
            options.l0nu
        };
        timing::sweeps(level, nu);
        workspaces.push(LevelWorkspace::<F> {
            nu,
            omega_override,
            e: if level == 0 { Vec::new() } else { vec![scalar::<F>(0.0); n] },
            d_prime: vec![scalar::<F>(0.0); n],
            d: if level == 0 { Vec::new() } else { vec![scalar::<F>(0.0); n] },
            smoother,
        });
    }
    workspaces
}

fn operator_to_dense<F: Real>(op: &dyn Operator<F>) -> Vec<F> {
    let n = op.n();
    let mut dense = vec![scalar::<F>(0.0); n * n];
    for row in 0..n {
        op.for_each_entry(row, &mut |col, val| dense[row * n + col] = val);
    }
    dense
}

fn build_coarse_levels<F: Real>(
    fine: &FineOperator<'_, F>,
    nrows: usize,
    ncols: usize,
    options: MgOptions,
) -> Vec<MgLevel<F>> {
    let mut coarse: Vec<MgLevel<F>> = Vec::new();
    let (mut current_rows, mut current_cols) = (nrows, ncols);

    while 1 + coarse.len() < options.max_levels {
        if options.cholesky_size.is_some_and(|size| current_rows * current_cols <= size) {
            break;
        }
        let fine_nr = current_rows;
        let fine_nc = current_cols;
        let next_nr = fine_nr.div_ceil(2);
        let next_nc = fine_nc.div_ceil(2);
        let min_side = if options.cholesky_size.is_some() {
            1
        } else {
            4
        };
        if next_nr < min_side || next_nc < min_side || (next_nr == current_rows && next_nc == current_cols) {
            break;
        }

        let prolongation = Transfer {
            fine_nrows: fine_nr,
            fine_ncols: fine_nc,
            coarse_nrows: next_nr,
            coarse_ncols: next_nc,
        };
        let laplacian = if coarse.is_empty() && matches!(fine, FineOperator::Stencil(_)) {
            // Keep the first Galerkin level implicit only when its source
            // is the five-point stencil; arbitrary explicit matrices need CSR.
            let matvec_workspace = RefCell::new(GalerkinMatvecWorkspace::new(fine.n()));
            let op = GalerkinOperator {
                fine,
                transfer: prolongation,
                reg_diag0: scalar::<F>(0.0),
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
                .map(|level| level.operator(fine))
                .unwrap_or(LevelOperator::Borrowed(fine));
            let mut matrix = galerkin_coarsen_operator(&previous, prolongation);
            crate::circuit::laplacian::regularize_laplacian(&mut matrix);
            CoarseOperator::Explicit(matrix)
        };
        coarse.push(MgLevel::<F> {
            laplacian,
            nrows: next_nr,
            ncols: next_nc,
            cholesky_l: None,
            prolongation,
        });

        timing::level(coarse.len(), next_nr, next_nc,
            match &coarse.last().unwrap().laplacian {
                CoarseOperator::Explicit(_) => "explicit",
                CoarseOperator::Galerkin { .. } => "galerkin-stencil",
            });
        current_rows = next_nr;
        current_cols = next_nc;
    }

    coarse
}

impl<F: Real> MgPreconditioner<'static, F> {
    /// Build a multigrid hierarchy from a materialised fine-grid matrix.
    pub fn build_from_laplacian(
        laplacian: CsMat<F>,
        nrows: usize,
        ncols: usize,
        max_levels: usize,
    ) -> Self {
        Self::build_from_operator(FineOperator::Explicit(laplacian), nrows, ncols, max_levels)
    }
}

impl<'a, F: Real> MgPreconditioner<'a, F> {
    /// Build a multigrid hierarchy from a fine-grid operator (materialised or
    /// matrix-free). A stencil fine grid also keeps level 1 matrix-free;
    /// subsequent levels are Galerkin-coarsened and materialised.
    pub fn build_from_operator(
        fine: FineOperator<'a, F>,
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
        fine: FineOperator<'a, F>,
        nrows: usize,
        ncols: usize,
        options: MgOptions,
    ) -> Self {
        assert!(nrows > 0 && ncols > 0, "MG requires nonempty dimensions");
        assert!(options.max_levels > 0, "MG requires at least one level");
        for omega in [options.l0omega, options.coarseomega].into_iter().flatten() {
            assert!(omega.is_finite() && omega > 0.0, "smoothing omega must be finite and positive");
        }
        assert!(options.l0nu > 0 && options.l1nu.unwrap_or(options.l0nu) > 0 && options.coarsenu > 0, "MG requires at least one smoothing sweep");
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

        timing::level(0, nrows, ncols, match &fine {
            FineOperator::Explicit(_) => "explicit",
            FineOperator::Stencil(_) => "stencil",
        });
        let mut coarse = build_coarse_levels(&fine, nrows, ncols, options);

        // Factor once during setup, then reuse for each V-cycle. If there is
        // no coarse level, only materialise a bounded small fine-grid system.
        let mut fine_cholesky_l = None;
        if let Some(coarsest) = coarse.last_mut() {
            let cnodes = coarsest.nrows * coarsest.ncols;
            memory::record_coarse_dense_typed::<F>(cnodes);
            let dense = operator_to_dense(&coarsest.operator(&fine));
            coarsest.cholesky_l = cholesky::cholesky_decompose(&dense, cnodes);
        } else if fine.n() <= options.cholesky_size.unwrap_or(SMALL_FINE_DIRECT_LIMIT) {
            let n = fine.n();
            memory::record_coarse_dense_typed::<F>(n);
            let dense = operator_to_dense(&fine);
            fine_cholesky_l = cholesky::cholesky_decompose(&dense, n);
        }
        memory::record_fine_operator(&fine);
        memory::record_coarsest_typed::<F>(
            coarse.last().map_or(nrows * ncols, |level| level.nrows * level.ncols),
            options.cholesky_size,
            coarse
                .last()
                .map(|level| level.cholesky_l.is_some())
                .unwrap_or(fine_cholesky_l.is_some()),
        );

        #[cfg(feature = "instrumentation-profile")]
        record_level_memory(&coarse, fine_cholesky_l.as_deref());

        let workspaces = build_level_workspaces(&fine, &coarse, options);

        Self {
            fine,
            coarse,
            fine_cholesky_l,
            omega: scalar::<F>(0.67),
            workspaces: RefCell::new(workspaces),
        }
    }

}
