use crate::real::{Real, scalar};
use crate::current::linalg::cholesky;
use crate::current::linalg::operator::{FineOperator, Operator, StencilOperator};
use crate::current::linalg::pcg::Preconditioner;
use crate::current::linalg::transfer::Transfer;
use crate::current::linalg::{timing, profile};
use crate::current::linalg::smooth::Smoother;
use sprs::CsMat;
use std::cell::RefCell;

mod galerkin;
mod setup;

use galerkin::{GalerkinMatvecWorkspace, GalerkinOperator};

/// Native multigrid setup controls; browser calls use the defaults.
#[derive(Clone, Copy, Debug)]
pub struct MgOptions {
    /// Sweeps in each pre/post phase on level 0.
    pub l0nu: usize,
    /// Level-1 sweeps; None inherits l0nu.
    pub l1nu: Option<usize>,
    /// Sweeps in each pre/post phase on deeper nonterminal levels.
    pub coarsenu: usize,
    /// Optional direct damping overrides on level 0 and levels 1 onward.
    pub l0omega: Option<f64>,
    pub coarseomega: Option<f64>,
    /// Print aggregate solve timings by level.
    pub verbose: bool,
    /// Trace per-iteration equation residuals in the native solve wrappers.
    pub profile: bool,
    /// Extra fine-level sweeps applied once after CG.
    pub l0postnu: usize,
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
            l0nu: 2,
            l1nu: Some(5),
            coarsenu: 25,
            l0omega: Some(0.95),
            coarseomega: Some(0.4),
            verbose: false,
            profile: false,
            l0postnu: 40,
            max_levels: 8,
            cholesky_size: None,
        }
    }
}

/// One (coarse) level of the multigrid hierarchy.
struct MgLevel<F: Real = f64> {
    laplacian: CoarseOperator<F>,
    nrows: usize,
    ncols: usize,
    cholesky_l: Option<Vec<F>>,
    /// Geometric transfer from this level to the finer level above.
    prolongation: Transfer,
}

/// Per-level scratch vectors used by the V-cycle:
/// `e` (error approximation), `d_prime` (residual of the error system),
/// `d` (the level's right-hand side).
struct LevelWorkspace<F: Real = f64> {
    nu: usize,
    omega_override: Option<F>,
    e: Vec<F>,
    d_prime: Vec<F>,
    d: Vec<F>,
    smoother: Smoother<F>,
}

// The finest level borrows its RHS and correction directly from CG.
struct CycleWorkspace<'a, F: Real = f64> {
    nu: usize,
    omega_override: Option<F>,
    e: &'a mut [F],
    d_prime: &'a mut [F],
    d: &'a [F],
    smoother: &'a Smoother<F>,
}

impl<F: Real> LevelWorkspace<F> {
    fn as_cycle(&mut self) -> CycleWorkspace<'_, F> {
        CycleWorkspace::<F> {
            nu: self.nu,
            omega_override: self.omega_override,
            e: &mut self.e,
            d_prime: &mut self.d_prime,
            d: &self.d,
            smoother: &self.smoother,
        }
    }
}

///
/// Level 0 (the fine grid) is an [`Operator`]: either a materialised CSR
/// matrix or a matrix-free stencil. The stencil path uses weighted Jacobi
/// and applies level 1 as P^T L_0 (P x); level 2 is the first CSR. Explicit MG
/// stores every Laplacian. Both paths use weighted Jacobi smoothing and
/// geometric transfers without entry arrays.
pub struct MgPreconditioner<'a, F: Real = f64> {
    fine: FineOperator<'a, F>,
    // Retain operators for post-smoothing on ascent and reuse across PCG iterations.
    coarse: Vec<MgLevel<F>>,
    fine_cholesky_l: Option<Vec<F>>,
    omega: F,
    // Preconditioner::apply takes &self; RefCell permits scratch-buffer reuse.
    // Borrow once at the boundary, then recurse with ordinary mutable slices.
    workspaces: RefCell<Vec<LevelWorkspace<F>>>,
}

/// Level 1 stores only its regularisation and structure count; deeper levels
/// retain CSR matrices. The fine operator is borrowed only by temporary views,
/// so the owning hierarchy never contains references into itself.
enum CoarseOperator<F: Real = f64> {
    Galerkin {
        reg_diag0: F,
        matvec_workspace: RefCell<GalerkinMatvecWorkspace<F>>,
        #[cfg(feature = "instrumentation-profile")]
        nnz: usize,
    },
    Explicit(CsMat<F>),
}

impl<F: Real> MgLevel<F> {
    fn operator<'b>(&'b self, fine: &'b FineOperator<'b, F>) -> LevelOperator<'b, F> {
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
enum LevelOperator<'b, F: Real = f64> {
    Borrowed(&'b dyn Operator<F>),
    Galerkin(GalerkinOperator<'b, FineOperator<'b, F>, F>),
}

impl<F: Real> Operator<F> for LevelOperator<'_, F> {
    fn n(&self) -> usize {
        match self {
            Self::Borrowed(op) => op.n(),
            Self::Galerkin(op) => op.n(),
        }
    }
    fn matvec(&self, x: &[F], y: &mut [F]) {
        match self {
            Self::Borrowed(op) => op.matvec(x, y),
            Self::Galerkin(op) => op.matvec(x, y),
        }
    }
    fn diag_inv(&self) -> Vec<F> {
        match self {
            Self::Borrowed(op) => op.diag_inv(),
            Self::Galerkin(op) => op.diag_inv(),
        }
    }
    fn for_each_entry(&self, row: usize, f: &mut dyn FnMut(usize, F)) {
        match self {
            Self::Borrowed(op) => op.for_each_entry(row, f),
            Self::Galerkin(op) => op.for_each_entry(row, f),
        }
    }
}

impl<'a, F: Real> MgPreconditioner<'a, F> {
    /// The fine-grid operator (level 0).
    pub fn fine_operator(&self) -> &dyn Operator<F> {
        &self.fine
    }

    pub(crate) fn fine_stencil(&self) -> &StencilOperator<'a, F> {
        match &self.fine {
            FineOperator::Stencil(op) => op,
            _ => unreachable!("mixed precision requires the low-memory stencil"),
        }
    }

    fn level_operator(&self, level: usize) -> LevelOperator<'_, F> {
        if level == 0 {
            LevelOperator::Borrowed(&self.fine)
        } else {
            self.coarse[level - 1].operator(&self.fine)
        }
    }

    fn v_cycle<const PROFILE: bool>(
        &self, mut current: CycleWorkspace<'_, F>, deeper: &mut [LevelWorkspace<F>], level: usize,
    ) {
        let op = self.level_operator(level);
        current.e.fill(scalar::<F>(0.0));
        if PROFILE {
            let cached = match current.smoother {
                Smoother::WeightedJacobi { diag_inv, omega } => Some((diag_inv.as_slice(), *omega)),
                _ => None,
            };
            profile::smoother_info(level, &op, cached);
            profile::level_sample(level, "entry", None, &op, current.d, current.e, true);
        }

        if deeper.is_empty() {
            let factor = if level == 0 {
                self.fine_cholesky_l.as_ref()
            } else {
                self.coarse[level - 1].cholesky_l.as_ref()
            };
            if let Some(factor) = factor {
                let _timer = timing::timer(Some(level), "Coarse direct solve");
                let error = cholesky::cholesky_solve(factor, current.d, current.e.len());
                current.e.copy_from_slice(&error);
            } else {
                // Start from zero so the result does not depend on an earlier
                // application when Cholesky is unavailable.
                let _timer = timing::timer(Some(level), "Coarse fallback CG");
                let result = crate::current::linalg::pcg::cg_solve(&op, current.d, 50, scalar::<F>(1e-3));
                current.e.copy_from_slice(&result.v);
            }
            if PROFILE {
                profile::level_sample(level, "terminal solve", None, &op, current.d, current.e, false);
            }
            return;
        }

        self.smooth_phase::<PROFILE>(&mut current, &op, level, false);
        let phase_timer = timing::timer(Some(level), "Residual calculation");
        op.matvec(current.e, current.d_prime);
        for (residual, &source) in current.d_prime.iter_mut().zip(current.d) {
            *residual = source - *residual;
        }

        drop(phase_timer);
        let transfer = self.coarse[level].prolongation;
        let phase_timer = timing::timer(Some(level), "Restriction");
        transfer.restrict(&current.d_prime, &mut deeper[0].d);
        drop(phase_timer);
        let (next, rest) = deeper.split_first_mut().unwrap();
        self.v_cycle::<PROFILE>(next.as_cycle(), rest, level + 1);
        let phase_timer = timing::timer(Some(level), "Prolongation/correction");
        transfer.prolongate(&deeper[0].e, current.e);
        drop(phase_timer);
        if PROFILE {
            profile::level_sample(level, "coarse correction", None, &op, current.d, current.e, false);
        }
        self.smooth_phase::<PROFILE>(&mut current, &op, level, true);
    }

    #[inline]
    fn smooth_phase<const PROFILE: bool>(
        &self, current: &mut CycleWorkspace<'_, F>, op: &dyn Operator<F>, level: usize, post: bool,
    ) {
        let name = if post { "Post-smoothing" } else { "Pre-smoothing" };
        if PROFILE {
            let mut elapsed = std::time::Duration::ZERO;
            for sweep in 0..current.nu {
                let start = std::time::Instant::now();
                current.smoother.smooth(op, current.e, current.d,
                    current.d_prime, current.omega_override.unwrap_or(self.omega));
                elapsed += start.elapsed();
                profile::level_sample(level, if post { "post-sweep" } else { "pre-sweep" },
                    Some((sweep + 1, current.nu)), op, current.d, current.e, false);
            }
            timing::record_duration(level, name, elapsed);
        } else {
            let _timer = timing::timer(Some(level), name);
            for _ in 0..current.nu {
                current.smoother.smooth(op, current.e, current.d,
                    current.d_prime, current.omega_override.unwrap_or(self.omega));
            }
        }
    }

    pub(crate) fn solve_cg(&self, rhs: &[F], max_iter: usize, tol: F, profile: bool)
        -> crate::current::linalg::pcg::CgResult::<F>
    {
        if profile {
            crate::current::linalg::pcg::cg_solve_precond_profiled(self.fine_operator(), rhs,
                max_iter, tol, &ProfiledPreconditioner::<F>(self))
        } else {
            crate::current::linalg::pcg::cg_solve_precond(self.fine_operator(), rhs, max_iter, tol, self)
        }
    }

    pub(crate) fn final_post_smooth(&self, rhs: &[F], x: &mut [F], count: usize,
        profiled: bool, iteration: usize)
    {
        if count == 0 { return; }
        if profiled {
            self.final_post_smooth_impl::<true>(rhs, x, count, iteration);
        } else {
            self.final_post_smooth_impl::<false>(rhs, x, count, iteration);
        }
    }

    fn final_post_smooth_impl<const PROFILE: bool>(&self, rhs: &[F], x: &mut [F],
        count: usize, iteration: usize)
    {
        let op = self.fine_operator();
        let mut workspaces = self.workspaces.borrow_mut();
        let current = &mut workspaces[0];
        if PROFILE {
            profile::global_sample(iteration, "Before final smoothing", op, rhs, x);
        }
        for sweep in 1..=count {
            let timer = timing::timer(Some(0), "Final post-smoothing");
            current.smoother.smooth(op, x, rhs, &mut current.d_prime, current.omega_override.unwrap_or(self.omega));
            drop(timer);
            if PROFILE {
                // Global samples retain the last CG residual as their baseline.
                profile::global_sweep_sample(iteration, "Final post-sweep", Some((sweep, count)), op, rhs, x);
            }
        }
    }

    fn apply_impl<const PROFILE: bool>(&self, r: &[F], e0: &mut Vec<F>) {
        assert_eq!(r.len(), self.fine.n());
        let mut workspaces = self.workspaces.borrow_mut();
        e0.resize(r.len(), scalar::<F>(0.0));
        let (fine, deeper) = workspaces.split_first_mut().unwrap();
        let mut current = fine.as_cycle();
        current.d = r;
        current.e = e0;
        self.v_cycle::<PROFILE>(current, deeper, 0);
    }

}

impl<F: Real> Preconditioner<F> for MgPreconditioner<'_, F> {
    fn apply(&self, r: &[F], e0: &mut Vec<F>) {
        self.apply_impl::<false>(r, e0);
    }
}

struct ProfiledPreconditioner<'m, 'a, F: Real = f64>(&'m MgPreconditioner<'a, F>);

impl<F: Real> Preconditioner<F> for ProfiledPreconditioner<'_, '_, F> {
    fn apply(&self, r: &[F], e0: &mut Vec<F>) {
        self.0.apply_impl::<true>(r, e0);
    }
}

#[cfg(test)]
mod tests;
