use sprs::CsMat;
use crate::linalg::pcg::Preconditioner;
use crate::linalg::cholesky;
use crate::linalg::operator::{FineOperator, Operator};
use crate::memory;
#[cfg(feature = "instrumentation-profile")]
use crate::memory::GalerkinScratch;
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
    laplacian: CsMat<f64>,
    nrows: usize,
    ncols: usize,
    cholesky_l: Option<Vec<f64>>,
    /// Prolongation triplets (rows, cols, vals) from this level to the finer level above.
    prolongation: Option<(Vec<usize>, Vec<usize>, Vec<f64>)>,
}

/// Per-level scratch vectors used by the V-cycle:
/// `e` (error approximation), `d_prime` (residual of the error system),
/// `d` (the level's right-hand side).
struct LevelWorkspace {
    e: Vec<f64>,
    d_prime: Vec<f64>,
    d: Vec<f64>,
}

/// Multigrid preconditioner: applies one V-cycle as `M⁻¹·r`.
///
/// Level 0 (the fine grid) is an [`Operator`]: either a materialised CSR
/// matrix or a matrix-free stencil. Deeper levels are materialised coarse
/// Laplacians produced by Galerkin coarsening.
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

/// Build a sparse prolongation matrix P: coarse → fine.
/// Uses bilinear interpolation: each coarse cell contributes to a 4×4
/// block of fine cells with weights derived from the tensor product of
/// linear interpolation in each dimension.
///
/// Row weights: [1/4, 3/4, 3/4, 1/4]  (centered at the coarse cell)
/// Col weights: [1/4, 3/4, 3/4, 1/4]
/// The 2-D weight is the product, giving 9/16 on the four center fine
/// cells, 3/16 on the eight edge neighbours, and 1/16 on the four corners.
///
/// P has dimensions (fine_nrows * fine_ncols) x (coarse_nrows * coarse_ncols).
/// Returns (row_indices, col_indices, values) triplets.
fn build_prolongation_triplets(
    fine_nrows: usize,
    fine_ncols: usize,
    coarse_nrows: usize,
    coarse_ncols: usize,
) -> (Vec<usize>, Vec<usize>, Vec<f64>) {
    let fine_n = fine_nrows * fine_ncols;
    let mut rows = Vec::with_capacity(fine_n * 4);
    let mut cols = Vec::with_capacity(fine_n * 4);
    let mut vals = Vec::with_capacity(fine_n * 4);

    let rw: [f64; 4] = [0.25, 0.75, 0.75, 0.25];
    let cw: [f64; 4] = [0.25, 0.75, 0.75, 0.25];
    let offsets: [isize; 4] = [-1, 0, 1, 2];

    for cr in 0..coarse_nrows {
        for cc in 0..coarse_ncols {
            let coarse_idx = cr * coarse_ncols + cc;

            for (ri, &ro) in offsets.iter().enumerate() {
                let fr = 2 * cr as isize + ro;
                if fr < 0 || fr >= fine_nrows as isize { continue; }
                let fr = fr as usize;

                for (ci, &co) in offsets.iter().enumerate() {
                    let fc = 2 * cc as isize + co;
                    if fc < 0 || fc >= fine_ncols as isize { continue; }
                    let fc = fc as usize;

                    rows.push(fr * fine_ncols + fc);
                    cols.push(coarse_idx);
                    vals.push(rw[ri] * cw[ci]);
                }
            }
        }
    }

    (rows, cols, vals)
}

/// Apply prolongation: fine += P * coarse
fn prolongate_sparse(rows: &[usize], cols: &[usize], vals: &[f64], coarse: &[f64], fine: &mut [f64]) {
    for k in 0..rows.len() {
        fine[rows[k]] += vals[k] * coarse[cols[k]];
    }
}

/// Apply restriction: coarse = P^T * fine
fn restrict_sparse(rows: &[usize], cols: &[usize], vals: &[f64], fine: &[f64], coarse: &mut [f64]) {
    coarse.fill(0.0);
    for k in 0..rows.len() {
        coarse[cols[k]] += vals[k] * fine[rows[k]];
    }
}

/// Galerkin coarse operator: `L_coarse = P^T * L_fine * P`.
/// Reads rows from any operator without materialising the intermediate `L_fine * P`.
fn galerkin_coarsen_operator(fine: &dyn Operator, p: &CsMat<f64>, coarse_n: usize) -> CsMat<f64> {
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
    /// matrix-free). Deeper levels are Galerkin-coarsened and materialised.
    pub fn build_from_operator(
        fine: FineOperator<'a>,
        nrows: usize,
        ncols: usize,
        max_levels: usize,
    ) -> Self {
        Self::build_with_options(fine, nrows, ncols, MgOptions {
            max_levels, ..MgOptions::default()
        })
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

            let coarse_n = next_nr * next_nc;
            let (p_rows, p_cols, p_vals) =
                build_prolongation_triplets(fine_nr, fine_nc, next_nr, next_nc);
            let fine_n = fine_nr * fine_nc;

            // Build prolongation as a sparse matrix for Galerkin
            let p_tri = sprs::TriMat::from_triplets(
                (fine_n, coarse_n),
                p_rows.clone(),
                p_cols.clone(),
                p_vals.clone(),
            );
            let p = p_tri.to_csr();

            // The first transition reads the original fine representation (CSR
            // or stencil); later transitions read the last retained coarse CSR.
            let previous: &dyn Operator = coarse
                .last()
                .map(|level| &level.laplacian as &dyn Operator)
                .unwrap_or(&fine);
            let mut laplacian = galerkin_coarsen_operator(previous, &p, coarse_n);
            crate::circuit::laplacian::regularize_laplacian(&mut laplacian);

            coarse.push(MgLevel {
                laplacian,
                nrows: next_nr,
                ncols: next_nc,
                cholesky_l: None,
                prolongation: Some((p_rows, p_cols, p_vals)),
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
            let dense = cholesky::sparse_to_dense(&coarsest.laplacian, cnodes);
            coarsest.cholesky_l = cholesky::cholesky_decompose(&dense, cnodes);
        } else if fine.n() <= options.cholesky_size.unwrap_or(SMALL_FINE_DIRECT_LIMIT) {
            let n = fine.n();
            memory::record_coarse_dense(n);
            let mut dense = vec![0.0; n * n];
            for row in 0..n {
                fine.for_each_entry(row, &mut |col, val| dense[row * n + col] = val);
            }
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
                let nnz = lvl.laplacian.nnz();
                let lap_bytes = memory::csmat_bytes(&lvl.laplacian);
                let prolongation_bytes = lvl
                    .prolongation
                    .as_ref()
                    .map(|(rows, _, _)| memory::triplets_bytes(rows.len()))
                    .unwrap_or(0);
                let cholesky_bytes = lvl
                    .cholesky_l
                    .as_ref()
                    .map(|l| memory::vec_f64_bytes(l.len()))
                    .unwrap_or(0);

                let (rows, _, _) = lvl.prolongation.as_ref().unwrap();
                let nnz_p = rows.len() as u64;
                let fine_n = if k == 0 {
                    nrows * ncols
                } else {
                    coarse[k - 1].nrows * coarse[k - 1].ncols
                };
                let u = memory::usize_size();
                let scratch = GalerkinScratch {
                    symbolic_seen_bytes: memory::vec_usize_bytes(nodes),
                    symbolic_row_nnz_bytes: memory::vec_usize_bytes(nodes),
                    numeric_pos_bytes: memory::vec_usize_bytes(nodes),
                    numeric_row_count_bytes: memory::vec_usize_bytes(nodes),
                    p_triplets_clone_bytes: memory::triplets_bytes(rows.len()),
                    p_csr_bytes: (fine_n as u64 + 1) * u + nnz_p * (u + 8),
                    p_csc_bytes: (nodes as u64 + 1) * u + nnz_p * (u + 8),
                };

                memory::record_level(
                    k + 1,
                    nodes,
                    nnz,
                    lap_bytes,
                    prolongation_bytes,
                    cholesky_bytes,
                    scratch,
                );
            }
        }

        // Pre-allocate scratch workspaces (one per level: fine + coarse)
        let mut workspaces = Vec::with_capacity(1 + coarse.len());
        workspaces.push(LevelWorkspace {
            e: vec![0.0; nrows * ncols],
            d_prime: vec![0.0; nrows * ncols],
            d: vec![0.0; nrows * ncols],
        });
        for lvl in &coarse {
            let n = lvl.nrows * lvl.ncols;
            workspaces.push(LevelWorkspace {
                e: vec![0.0; n],
                d_prime: vec![0.0; n],
                d: vec![0.0; n],
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

    fn level_operator(&self, level: usize) -> &dyn Operator {
        if level == 0 {
            &self.fine
        } else {
            &self.coarse[level - 1].laplacian
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
                let result = crate::linalg::pcg::cg_solve(op, &current.d, 50, 1e-3, None);
                current.e.copy_from_slice(&result.v);
            }
            return;
        }

        for _ in 0..self.nu {
            symmetric_gauss_seidel_smooth(op, &mut current.e, &current.d, self.omega);
        }
        op.matvec(&current.e, &mut current.d_prime);
        for (residual, &source) in current.d_prime.iter_mut().zip(&current.d) {
            *residual = source - *residual;
        }

        let (rows, cols, vals) = self.coarse[level].prolongation.as_ref().unwrap();
        restrict_sparse(rows, cols, vals, &current.d_prime, &mut deeper[0].d);
        self.v_cycle(deeper, level + 1);
        prolongate_sparse(rows, cols, vals, &deeper[0].e, &mut current.e);

        for _ in 0..self.nu {
            symmetric_gauss_seidel_smooth(op, &mut current.e, &current.d, self.omega);
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

fn symmetric_gauss_seidel_smooth(
    op: &dyn Operator,
    e: &mut [f64],
    d: &[f64],
    omega: f64,
) {
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
    fn build_mg(resistance: &[f64], nrows: usize, ncols: usize, max_levels: usize) -> MgPreconditioner<'static> {
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

    fn level_operator<'x>(mg: &'x MgPreconditioner<'_>, level: usize) -> &'x dyn Operator {
        if level == 0 {
            &mg.fine
        } else {
            &mg.coarse[level - 1].laplacian
        }
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
                for level in &profile.hierarchy[1..] {
                    // CSR output arrays are retained in the Laplacian, not scratch.
                    assert_eq!(
                        level.laplacian_bytes,
                        memory::vec_usize_bytes(level.nodes + 1)
                            + memory::vec_usize_bytes(level.nnz)
                            + memory::vec_f64_bytes(level.nnz)
                    );
                    assert!(level.galerkin_scratch.p_triplets_clone_bytes > 0);
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
            }
            let source: Vec<f64> = (0..nr * nc).map(|i| 1.0 + (i as f64).sin()).collect();
            let (mut first, mut again, mut reference) = (Vec::new(), Vec::new(), Vec::new());
            stencil.apply(&source, &mut first);
            stencil.apply(&vec![0.0; nr * nc], &mut again);
            stencil.apply(&source, &mut again);
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
                mg_e.coarse[0].laplacian.nnz(),
                mg_s.coarse[0].laplacian.nnz(),
                "{tag}: coarse level-1 nnz differs"
            );
            let mut ze = vec![0.0; n];
            let mut zs = vec![0.0; n];
            mg_e.apply(&x, &mut ze);
            mg_s.apply(&x, &mut zs);
            for i in 0..n {
                assert!(
                    (ze[i] - zs[i]).abs() < 1e-6,
                    "{tag}: MG stencil vs explicit mismatch at {}: {} vs {}",
                    i, ze[i], zs[i]
                );
            }
        };

        // No ground
        {
            let mg_e = MgPreconditioner::build_from_laplacian(lap.clone(), nrows, ncols, 5);
            let st = StencilOperator::new(nrows, ncols, nodata, &resistance, GroundSpec::None);
            let mg_s = MgPreconditioner::build_from_operator(FineOperator::Stencil(st), nrows, ncols, 5);
            check(&mg_e, &mg_s, "none");
        }

        // Neumann ground
        {
            let a = crate::circuit::laplacian::add_diagonal(&lap, &shunt);
            let mg_e = MgPreconditioner::build_from_laplacian(a, nrows, ncols, 5);
            let st = StencilOperator::new(nrows, ncols, nodata, &resistance, GroundSpec::Neumann(shunt.clone()));
            let mg_s = MgPreconditioner::build_from_operator(FineOperator::Stencil(st), nrows, ncols, 5);
            check(&mg_e, &mg_s, "neumann");
        }

        // Dirichlet ground
        {
            let a = crate::solve::apply_dirichlet_ground_lap(&lap, &gns);
            let mg_e = MgPreconditioner::build_from_laplacian(a, nrows, ncols, 5);
            let st = StencilOperator::new(nrows, ncols, nodata, &resistance, GroundSpec::Dirichlet(mask.clone()));
            let mg_s = MgPreconditioner::build_from_operator(FineOperator::Stencil(st), nrows, ncols, 5);
            check(&mg_e, &mg_s, "dirichlet");
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
        assert!((dot_x_ay - dot_y_ax).abs() < 1e-7, "Fine matrix A is asymmetric!");

        mg.apply(&x, &mut mx);
        mg.apply(&y, &mut my);
        
        // 3. Compute dot products: x · M(y) vs y · M(x)
        let dot_x_my: f64 = x.iter().zip(my.iter()).map(|(a, b)| a * b).sum();
        let dot_y_mx: f64 = y.iter().zip(mx.iter()).map(|(a, b)| a * b).sum();
        
        // Operators must be symmetric up to floating-point drift
        assert!(
            (dot_x_my - dot_y_mx).abs() < 1e-6, 
            "MG Preconditioner is asymmetric! x*M(y) = {}, y*M(x) = {}", dot_x_my, dot_y_mx
        );
    }

    #[test]
    fn test_mg_preconditioned_cg_converges() {
        let nrows = 64;
        let ncols = 64;
        let n = nrows * ncols;

        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = build_mg(&resistance, nrows, ncols, 7);

        let b_raw: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5).collect();
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
        eprintln!("Large uniform {}x{}: {} iters, ||r|| went from {:.6e} to {:.6e} (ratio {:.4e})",
            nrows, ncols, residuals.len(), r0, r_last, r_last / r0);
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
                            level, row, val
                        );
                        let diag_inv = if val.abs() > 1e-15 { 1.0 / val.abs() } else { 0.0 };
                        assert!(
                            diag_inv > 0.0,
                            "Level {}: diag_inv[{}] = {:.6e} (must be positive)",
                            level, row, diag_inv
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

        let z: Vec<f64> = (0..n).map(|i| (i as f64 * 0.7 + 1.3).sin() * 0.5 + 0.3).collect();
        let mut mz = vec![0.0; n];
        mg.apply(&z, &mut mz);

        let z_mz: f64 = z.iter().zip(mz.iter()).map(|(a, b)| a * b).sum();
        assert!(
            z_mz > 0.0,
            "Preconditioner is not positive definite: zᵀ·M⁻¹·z = {:.6e}",
            z_mz
        );

        let z2: Vec<f64> = (0..n).map(|i| (i as f64 * 1.1 + 2.7).cos() * 0.8 - 0.2).collect();
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

        let b_raw: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5).collect();
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

        let b: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.5 + 0.3).sin() + 1.0) * 0.5).collect();
        let l = lvl.cholesky_l.as_ref().unwrap();
        let z = crate::linalg::cholesky::cholesky_solve(l, &b, n);

        let mut az = vec![0.0; n];
        pcg::mat_vec_mul_into(&lvl.laplacian, &z, &mut az);

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
            let mut x: Vec<f64> = (0..n).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();

            let initial_norm = x.iter().map(|v| v * v).sum::<f64>().sqrt();
            for _ in 0..3 {
                symmetric_gauss_seidel_smooth(level_operator(&mg, level), &mut x, &b, mg.omega);
            }
            let final_norm = x.iter().map(|v| v * v).sum::<f64>().sqrt();

            assert!(
                final_norm < initial_norm,
                "Level {}: smoother did not reduce error norm (initial={:.6e}, final={:.6e})",
                level, initial_norm, final_norm
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

        let b_raw: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5).collect();
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
        eprintln!("  ||b|| = {:.6e}, r·z = {:.6e}, z·Az = {:.6e}", b_norm, rz, z_az);

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
            eprintln!("Level {}: diag_inv range = [{:.6e}, {:.6e}] (ratio = {:.2e})",
                level, min_inv, max_inv, max_inv / min_inv.max(1e-30));

            assert!(
                min_inv > 1e-15,
                "Level {}: diag_inv too small: min = {:.6e}",
                level, min_inv
            );
            assert!(
                max_inv < 1e10,
                "Level {}: diag_inv too large: max = {:.6e}",
                level, max_inv
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
        let b: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.5 + 0.3).sin() + 1.0) * 0.5).collect();
        let x = crate::linalg::cholesky::cholesky_solve(l, &b, n);

        let mut ax = vec![0.0; n];
        pcg::mat_vec_mul_into(&lvl.laplacian, &x, &mut ax);

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
                if d < min_d { min_d = d; }
                if d > max_d { max_d = d; }
            }
            eprintln!("Level {}: diag_inv [{:.4e}, {:.4e}] ratio {:.2e}",
                level, min_d, max_d, max_d / min_d.max(1e-30));
        }

        let b_raw: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5).collect();
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
            eprintln!("  iter {}: ||r||={:.4e} r·z={:.4e} z·Az={:.4e}",
                _iter, r_norm, rz, z_az);

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
        eprintln!("Variable coeff {}x{}: {} iters, ||r|| went from {:.6e} to {:.6e} (ratio {:.4e})",
            nrows, ncols, residuals.len(), r0, r_last, r_last / r0);
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

        let b_raw: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5).collect();
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
        eprintln!("Smoother-only {}x{}: {} iters, ||r|| went from {:.6e} to {:.6e} (ratio {:.4e})",
            nrows, ncols, residuals.len(), r0, r_last, r_last / r0);
    }
}
