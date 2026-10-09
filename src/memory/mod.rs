//! Analytical instrumentation of the connectivity solver's named buffers.
//!
//! Reports matrix/vector payload sizes using lengths and structural entry
//! counts. These estimates exclude allocation capacity, container overhead,
//! caller-owned inputs, raster preparation, returned maps, and warm-start copies.
//! Setup buffers are reported separately from retained solver structures;
//! summing them does not measure concurrently live memory or a heap peak.
//! Boundary reconstruction and sparse-format conversion internals are excluded;
//! setup fields cover the named buffers, not every allocation in dependencies.
//!
//! Enable with `cargo test --features instrumentation-profile` or run the native
//! harness with `cargo run --profile release-prof --features bin,instrumentation-profile
//! --bin instrumentation-profile -- 500`. Without that Cargo feature, recording
//! and reset bodies are no-ops and `take_profile` returns None.

use serde::Serialize;
use sprs::CsMat;

/// Byte width of a `usize`. Native (64-bit) is 8; `wasm32` is 4.
#[inline]
pub fn usize_size() -> u64 {
    std::mem::size_of::<usize>() as u64
}

/// Bytes of a `Vec<f64>` of length `n`.
#[inline]
pub fn vec_f64_bytes(n: usize) -> u64 {
    n as u64 * 8
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
pub fn csmat_bytes(a: &CsMat<f64>) -> u64 {
    let u = usize_size();
    (a.rows() as u64 + 1) * u + a.nnz() as u64 * u + a.nnz() as u64 * 8
}

/// Bytes of `n` triplets stored as three parallel vectors
/// (`row: Vec<usize>`, `col: Vec<usize>`, `val: Vec<f64>`).
#[inline]
pub fn triplets_bytes(n: usize) -> u64 {
    n as u64 * (2 * usize_size() + 8)
}

/// Bytes of a dense `n x n` `f64` matrix.
#[inline]
pub fn dense_bytes(n: usize) -> u64 {
    n as u64 * n as u64 * 8
}

/// Temporary Galerkin-coarsening buffers used to build a coarse level
/// `L_{l+1} = P_l^T L_l P_l`.
#[derive(Serialize, Clone, Default, Debug)]
pub struct GalerkinScratch {
    /// `seen`, reused by both passes, length `coarse_n`.
    pub symbolic_seen_bytes: u64,
    /// `row_nnz` (pass 1), length `coarse_n`.
    pub symbolic_row_nnz_bytes: u64,
    /// `pos` (pass 2), length `coarse_n`.
    pub numeric_pos_bytes: u64,
    /// `row_count` (pass 2), length `coarse_n`.
    pub numeric_row_count_bytes: u64,
    /// Cloned prolongation triplets used to construct the temporary CSR.
    pub p_triplets_clone_bytes: u64,
    /// Prolongation `P_l` materialised as CSR.
    pub p_csr_bytes: u64,
    /// Prolongation transposed (`P_l` as CSC, i.e. the restriction `R_l`).
    pub p_csc_bytes: u64,
}

/// Memory footprint of one level of the geometric multigrid hierarchy.
/// Level 0 is the finest grid (its Laplacian is the system matrix `L`).
#[derive(Serialize, Clone, Default, Debug)]
pub struct LevelMemory {
    /// Level index; 0 is the finest grid.
    pub level: usize,
    /// Number of nodes `n_l` on this level.
    pub nodes: usize,
    /// Structural entries in `L_l`, including boundary/ground effects.
    /// Stencil entries are computed on demand and occupy zero matrix bytes.
    pub nnz: usize,
    /// Bytes of this level's Laplacian `L_l`.
    pub laplacian_bytes: u64,
    /// Bytes of the prolongation triplets `P_l` (empty for level 0).
    pub prolongation_p_bytes: u64,
    /// Bytes of the dense Cholesky factor (coarsest level only).
    pub cholesky_factor_bytes: u64,
    /// Bytes of the workspace vector `e_l`.
    pub workspace_e_bytes: u64,
    /// Bytes of the workspace vector `d_l`.
    pub workspace_d_bytes: u64,
    /// Bytes of the workspace vector `d'_l`.
    pub workspace_d_prime_bytes: u64,
    /// Freed Galerkin scratch used to build this level (empty for level 0).
    pub galerkin_scratch: GalerkinScratch,
}

/// Analytical solver payload profile. Hierarchy level 0 repeats the top-level
/// matrix size; count it only once when calculating totals.
#[derive(Serialize, Clone, Default, Debug)]
pub struct InstrumentationProfile {
    // ---- assembly buffers (not a concurrent peak) ----
    /// Conductance grid `C = 1/R` (per cell).
    pub conductance_grid_bytes: u64,
    /// Cell-to-node map (per cell, `i32`).
    pub cell_to_node_map_bytes: u64,
    /// Edge triplets (symmetric conductance pairs).
    pub edge_triplets_bytes: u64,
    pub assembly_row_sums_bytes: u64,
    pub assembly_laplacian_triplets_bytes: u64,
    /// Filled resistance copy used by the explicit solve.
    pub filled_resistance_bytes: u64,
    /// Ground construction buffers in the explicit solve (setup category).
    pub ground_setup_bytes: u64,
    /// Owned stencil ground shunts or mask; excludes the borrowed raster.
    pub ground_storage_bytes: u64,

    // ---- fine graph Laplacian L (the system matrix) ----
    pub laplacian_nodes: usize,
    /// Structural entry count; stencil entries occupy zero stored matrix bytes.
    pub laplacian_nnz: usize,
    pub laplacian_bytes: u64,
    /// How the fine Laplacian is represented: `"explicit"` (CSR) or
    /// `"stencil"` (matrix-free). Empty if not yet recorded.
    pub operator_repr: String,

    // ---- multigrid hierarchy (level 0 = fine, then coarser) ----
    pub hierarchy: Vec<LevelMemory>,

    // ---- coarsest-level direct solve ----
    /// Dense coarsest matrix built before factorisation (freed).
    pub coarse_dense_bytes: u64,
    pub requested_cholesky_size: Option<usize>,
    pub coarsest_nodes: usize,
    pub cholesky_available: bool,
    /// Temporary forward/back substitution outputs per direct solve.
    pub cholesky_solve_scratch_bytes: u64,
    /// CG/Jacobi buffers when direct factorization is unavailable.
    pub coarse_fallback_scratch_bytes: u64,

    // ---- CG workspace vectors (persistent through the solve) ----
    /// Voltage `v`.
    pub voltage_v_bytes: u64,
    /// Residual `r`.
    pub residual_r_bytes: u64,
    /// Preconditioned residual `e0`.
    pub preconditioned_residual_e0_bytes: u64,
    /// Search direction `p`.
    pub search_direction_p_bytes: u64,
    /// Matvec scratch `L p`.
    pub matvec_lp_bytes: u64,
    /// Source vector `s` (the right-hand side).
    pub source_s_bytes: u64,
    /// Jacobi diagonal `diag(L)` inverse (Jacobi path only).
    pub jacobi_diag_bytes: u64,

    // ---- warm-start cache (cached Jacobi path) ----
    pub cache_laplacian_bytes: u64,
    pub cache_cell_to_node_bytes: u64,
    pub cache_last_voltages_bytes: u64,
}

#[cfg(feature = "instrumentation-profile")]
mod imp {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static PROFILE: RefCell<Option<InstrumentationProfile>> = RefCell::new(None);
    }

    pub(super) fn with_profile<F: FnOnce(&mut InstrumentationProfile)>(f: F) {
        PROFILE.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                *slot = Some(InstrumentationProfile::default());
            }
            f(slot.as_mut().unwrap());
        });
    }

    pub fn reset() {
        PROFILE.with(|slot| *slot.borrow_mut() = None);
    }

    pub fn take() -> Option<InstrumentationProfile> {
        PROFILE.with(|slot| slot.borrow_mut().take())
    }
}

/// Clear the in-progress memory profile. Call at the start of a solve so each
/// run reports a fresh profile.
#[inline]
pub fn reset() {
    #[cfg(feature = "instrumentation-profile")]
    imp::reset();
}

/// Take (and clear) the accumulated memory profile, if the feature is enabled.
#[inline]
pub fn take_profile() -> Option<InstrumentationProfile> {
    #[cfg(feature = "instrumentation-profile")]
    {
        imp::take()
    }
    #[cfg(not(feature = "instrumentation-profile"))]
    {
        None
    }
}

// ---- assembly ----

#[inline]
#[allow(unused_variables)]
pub fn record_conductance_grid(n_cells: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| s.conductance_grid_bytes = vec_f64_bytes(n_cells));
}

#[inline]
#[allow(unused_variables)]
pub fn record_cell_to_node_map(n_cells: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| s.cell_to_node_map_bytes = vec_i32_bytes(n_cells));
}

#[inline]
#[allow(unused_variables)]
pub fn record_edge_triplets(n_triplets: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| s.edge_triplets_bytes = triplets_bytes(n_triplets));
}

// ---- fine Laplacian ----

#[inline]
#[allow(unused_variables)]
pub fn record_fine_laplacian(laplacian: &CsMat<f64>) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| {
        s.laplacian_nodes = laplacian.rows();
        s.laplacian_nnz = laplacian.nnz();
        s.laplacian_bytes = csmat_bytes(laplacian);
        s.operator_repr = "explicit".to_string();
    });
}

/// Record stencil structure without claiming stored matrix entries or bytes.
#[inline]
#[allow(unused_variables)]
pub fn record_stencil_fine(operator: &crate::linalg::operator::StencilOperator<'_>) {
    #[cfg(feature = "instrumentation-profile")]
    {
        use crate::linalg::operator::Operator;
        let mut entries = 0;
        for row in 0..operator.n() {
            operator.for_each_entry(row, &mut |_, _| entries += 1);
        }
        imp::with_profile(|profile| {
            profile.laplacian_nodes = operator.n();
            profile.laplacian_nnz = entries;
            profile.laplacian_bytes = 0;
            profile.ground_storage_bytes = operator.ground_storage_bytes();
            profile.operator_repr = "stencil".to_string();
        });
    }
}

#[inline]
#[allow(unused_variables)]
pub fn record_fine_operator(operator: &crate::linalg::operator::FineOperator<'_>) {
    #[cfg(feature = "instrumentation-profile")]
    match operator {
        crate::linalg::operator::FineOperator::Explicit(matrix) => record_fine_laplacian(matrix),
        crate::linalg::operator::FineOperator::Stencil(stencil) => record_stencil_fine(stencil),
    }
}

#[inline]
#[allow(unused_variables)]
pub fn record_assembly_scratch(nodes: usize, entries: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| {
        profile.assembly_row_sums_bytes = vec_f64_bytes(nodes);
        profile.assembly_laplacian_triplets_bytes = triplets_bytes(entries);
    });
}

#[inline]
#[allow(unused_variables)]
pub fn record_filled_resistance(cells: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| profile.filled_resistance_bytes = vec_f64_bytes(cells));
}

#[inline]
#[allow(unused_variables)]
pub fn record_ground_setup(bytes: u64) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| profile.ground_setup_bytes = bytes);
}

// ---- hierarchy ----

#[inline]
#[allow(unused_variables)]
pub fn record_level(
    level: usize,
    nodes: usize,
    nnz: usize,
    laplacian_bytes: u64,
    prolongation_p_bytes: u64,
    cholesky_factor_bytes: u64,
    galerkin_scratch: GalerkinScratch,
) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| {
        let workspace = vec_f64_bytes(nodes);
        s.hierarchy.push(LevelMemory {
            level,
            nodes,
            nnz,
            laplacian_bytes,
            prolongation_p_bytes,
            cholesky_factor_bytes,
            workspace_e_bytes: workspace,
            workspace_d_bytes: workspace,
            workspace_d_prime_bytes: workspace,
            galerkin_scratch,
        });
    });
}

/// Include fine-grid scratch vectors even when its matrix is a stencil.
#[inline]
#[allow(unused_variables)]
pub fn record_fine_level(cholesky_factor_bytes: u64) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| {
        let workspace = vec_f64_bytes(profile.laplacian_nodes);
        profile.hierarchy.push(LevelMemory {
            level: 0,
            nodes: profile.laplacian_nodes,
            nnz: profile.laplacian_nnz,
            laplacian_bytes: profile.laplacian_bytes,
            cholesky_factor_bytes,
            workspace_e_bytes: workspace,
            workspace_d_bytes: workspace,
            workspace_d_prime_bytes: workspace,
            ..LevelMemory::default()
        });
    });
}

// ---- coarsest direct solve ----

#[inline]
#[allow(unused_variables)]
pub fn record_coarsest(nodes: usize, requested: Option<usize>, cholesky_available: bool) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| {
        profile.coarsest_nodes = nodes;
        profile.requested_cholesky_size = requested;
        profile.cholesky_available = cholesky_available;
        if cholesky_available {
            profile.cholesky_solve_scratch_bytes = 2 * vec_f64_bytes(nodes);
        } else {
            profile.coarse_fallback_scratch_bytes = 6 * vec_f64_bytes(nodes);
        }
    });
}

#[inline]
#[allow(unused_variables)]
pub fn record_coarse_dense(n: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| s.coarse_dense_bytes = dense_bytes(n));
}

// ---- CG vectors ----

#[inline]
#[allow(unused_variables)]
pub fn record_cg_vectors(n: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| {
        let bytes = vec_f64_bytes(n);
        s.voltage_v_bytes = bytes;
        s.residual_r_bytes = bytes;
        s.preconditioned_residual_e0_bytes = bytes;
        s.search_direction_p_bytes = bytes;
        s.matvec_lp_bytes = bytes;
        s.source_s_bytes = bytes;
    });
}

#[inline]
#[allow(unused_variables)]
pub fn record_jacobi_diag(n: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| s.jacobi_diag_bytes = vec_f64_bytes(n));
}

// ---- cache ----

#[inline]
#[allow(unused_variables)]
pub fn record_cache_laplacian(a: &CsMat<f64>) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| s.cache_laplacian_bytes = csmat_bytes(a));
}

#[inline]
#[allow(unused_variables)]
pub fn record_cache_cell_to_node(n_cells: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| s.cache_cell_to_node_bytes = vec_i32_bytes(n_cells));
}

#[inline]
#[allow(unused_variables)]
pub fn record_cache_last_voltages(n: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| s.cache_last_voltages_bytes = vec_f64_bytes(n));
}

#[cfg(test)]
mod tests {
    use super::*;
    use sprs::TriMat;

    fn small_csmat() -> CsMat<f64> {
        let mut t = TriMat::new((3, 3));
        t.add_triplet(0, 0, 2.0);
        t.add_triplet(1, 1, 3.0);
        t.add_triplet(2, 2, 4.0);
        t.add_triplet(0, 1, 1.0);
        t.add_triplet(1, 0, 1.0);
        t.to_csr()
    }

    #[test]
    fn test_vec_sizers() {
        assert_eq!(vec_f64_bytes(10), 80);
        assert_eq!(vec_i32_bytes(10), 40);
        assert_eq!(vec_usize_bytes(10), 10 * usize_size());
        assert_eq!(dense_bytes(4), 4 * 4 * 8);
    }

    #[test]
    fn test_csmat_bytes_formula() {
        let a = small_csmat();
        // rows=3, nnz=5 -> (3+1)*u + 5*u + 5*8
        let expected = (3 + 1) * usize_size() + 5 * usize_size() + 5 * 8;
        assert_eq!(csmat_bytes(&a), expected);
    }

    #[test]
    fn test_triplets_bytes() {
        assert_eq!(triplets_bytes(7), 7 * (2 * usize_size() + 8));
    }

    #[test]
    fn test_take_profile_none_by_default() {
        // A fresh thread (no records) should return None.
        reset();
        assert!(take_profile().is_none());
        #[cfg(not(feature = "instrumentation-profile"))]
        {
            record_cg_vectors(3);
            record_fine_laplacian(&small_csmat());
            assert!(take_profile().is_none(), "disabled recording must be a no-op");
        }
    }

    #[cfg(feature = "instrumentation-profile")]
    #[test]
    fn test_record_accumulates() {
        reset();
        record_conductance_grid(100);
        record_cell_to_node_map(100);
        let a = small_csmat();
        record_fine_laplacian(&a);

        let profile = take_profile().expect("feature enabled -> profile present");
        assert_eq!(profile.conductance_grid_bytes, vec_f64_bytes(100));
        assert_eq!(profile.cell_to_node_map_bytes, vec_i32_bytes(100));
        assert_eq!(profile.laplacian_nodes, 3);
        assert_eq!(profile.laplacian_nnz, 5);
        assert_eq!(profile.laplacian_bytes, csmat_bytes(&a));

        // take() clears the profile
        assert!(take_profile().is_none());
    }

    #[cfg(feature = "instrumentation-profile")]
    #[test]
    fn test_record_level_workspace_sizing() {
        reset();
        record_level(1, 64, 9, 1000, 500, 0, GalerkinScratch::default());
        let profile = take_profile().unwrap();
        let lvl = &profile.hierarchy[0];
        assert_eq!(lvl.level, 1);
        assert_eq!(lvl.nodes, 64);
        assert_eq!(lvl.workspace_e_bytes, vec_f64_bytes(64));
        assert_eq!(lvl.workspace_d_bytes, vec_f64_bytes(64));
        assert_eq!(lvl.workspace_d_prime_bytes, vec_f64_bytes(64));
    }

    #[cfg(feature = "instrumentation-profile")]
    #[test]
    fn test_record_stencil_fine_zero_bytes() {
        reset();
        use crate::linalg::operator::{GroundSpec, StencilOperator};
        let resistance = vec![1.0; 100];
        let mut mask = vec![false; 100];
        mask[0] = true;
        let operator = StencilOperator::new(10, 10, -9999.0, &resistance,
            GroundSpec::Dirichlet(mask));
        record_stencil_fine(&operator);
        record_fine_level(0);
        let profile = take_profile().unwrap();
        assert_eq!(profile.operator_repr, "stencil");
        assert_eq!(profile.laplacian_nodes, 100);
        // 100 diagonal entries + 360 directed edges - 4 edges touching ground.
        assert_eq!(profile.laplacian_nnz, 456);
        assert_eq!(profile.laplacian_bytes, 0);
        assert_eq!(profile.ground_storage_bytes, 100);
        assert_eq!(profile.hierarchy[0].level, 0);
        assert_eq!(profile.hierarchy[0].laplacian_bytes, 0);
        assert_eq!(profile.hierarchy[0].workspace_e_bytes, 800);
    }

    #[cfg(feature = "instrumentation-profile")]
    #[test]
    fn test_record_fine_laplacian_explicit_repr() {
        reset();
        let a = small_csmat();
        record_fine_laplacian(&a);
        record_fine_level(0);
        let profile = take_profile().unwrap();
        assert_eq!(profile.operator_repr, "explicit");
        assert_eq!(profile.hierarchy[0].level, 0);
        assert_eq!(profile.hierarchy[0].laplacian_bytes, csmat_bytes(&a));
    }
}
