//! Feature-gated memory accounting for the connectivity solve.
//!
//! Reports the byte size of every data structure the algorithm allocates,
//! using the paper's naming: the graph Laplacian `L`, the coarse Laplacians
//! `L_l`, the prolongation operator `P_l` (with restriction `R_l = P_l^T`),
//! the per-level workspace vectors `e_l`, `d_l`, `d'_l`, the coarsest-level
//! Cholesky factor, and the CG vectors `v`, `r`, `e0`, `p`, `s`.
//!
//! Sizes are *analytical*: they are computed from each structure's dimensions
//! (node counts, stored-entry counts, vector lengths), not from a heap
//! profiler. Freed intermediates (conductance grid, cell-to-node map, edge
//! triplets, Galerkin scratch buffers, the dense coarsest matrix) are reported
//! too, so the story accounts for memory the algorithm touches and releases
//! along the way.
//!
//! Compile-time toggled by the `memory-story` feature. When the feature is
//! disabled, every `record_*` call is a no-op and `take_story` returns `None`,
//! so there is zero runtime and (effectively) zero binary overhead.

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

/// Freed Galerkin-coarsening scratch buffers used to build a coarse level
/// `L_{l+1} = P_l^T L_l P_l`.
#[derive(Serialize, Clone, Default, Debug)]
pub struct GalerkinScratch {
    /// `seen` (pass 1), length `coarse_n`.
    pub symbolic_seen_bytes: u64,
    /// `row_nnz` (pass 1), length `coarse_n`.
    pub symbolic_row_nnz_bytes: u64,
    /// `seen` (pass 2), length `coarse_n`.
    pub numeric_seen_bytes: u64,
    /// `pos` (pass 2), length `coarse_n`.
    pub numeric_pos_bytes: u64,
    /// `row_count` (pass 2), length `coarse_n`.
    pub numeric_row_count_bytes: u64,
    /// `indptr`, length `coarse_n + 1`.
    pub numeric_indptr_bytes: u64,
    /// `cols`, length `nnz`.
    pub numeric_cols_bytes: u64,
    /// `vals`, length `nnz`.
    pub numeric_vals_bytes: u64,
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
    /// Stored (non-zero) entries in this level's Laplacian `L_l`.
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

/// Complete memory story of one solve, one field per data structure.
#[derive(Serialize, Clone, Default, Debug)]
pub struct MemoryStory {
    // ---- assembly intermediates (freed once L is built) ----
    /// Conductance grid `C = 1/R` (per cell).
    pub conductance_grid_bytes: u64,
    /// Cell-to-node map (per cell, `i32`).
    pub cell_to_node_map_bytes: u64,
    /// Edge triplets (symmetric conductance pairs).
    pub edge_triplets_bytes: u64,

    // ---- fine graph Laplacian L (the system matrix) ----
    pub laplacian_nodes: usize,
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

#[cfg(feature = "memory-story")]
mod imp {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static STORY: RefCell<Option<MemoryStory>> = RefCell::new(None);
    }

    pub(super) fn with_story<F: FnOnce(&mut MemoryStory)>(f: F) {
        STORY.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                *slot = Some(MemoryStory::default());
            }
            f(slot.as_mut().unwrap());
        });
    }

    pub fn reset() {
        STORY.with(|slot| *slot.borrow_mut() = None);
    }

    pub fn take() -> Option<MemoryStory> {
        STORY.with(|slot| slot.borrow_mut().take())
    }
}

/// Clear the in-progress memory story. Call at the start of a solve so each
/// run reports a fresh story.
#[inline]
pub fn reset() {
    #[cfg(feature = "memory-story")]
    imp::reset();
}

/// Take (and clear) the accumulated memory story, if the feature is enabled.
#[inline]
pub fn take_story() -> Option<MemoryStory> {
    #[cfg(feature = "memory-story")]
    {
        imp::take()
    }
    #[cfg(not(feature = "memory-story"))]
    {
        None
    }
}

// ---- assembly ----

#[inline]
#[allow(unused_variables)]
pub fn record_conductance_grid(n_cells: usize) {
    #[cfg(feature = "memory-story")]
    imp::with_story(|s| s.conductance_grid_bytes = vec_f64_bytes(n_cells));
}

#[inline]
#[allow(unused_variables)]
pub fn record_cell_to_node_map(n_cells: usize) {
    #[cfg(feature = "memory-story")]
    imp::with_story(|s| s.cell_to_node_map_bytes = vec_i32_bytes(n_cells));
}

#[inline]
#[allow(unused_variables)]
pub fn record_edge_triplets(n_triplets: usize) {
    #[cfg(feature = "memory-story")]
    imp::with_story(|s| s.edge_triplets_bytes = triplets_bytes(n_triplets));
}

// ---- fine Laplacian ----

#[inline]
#[allow(unused_variables)]
pub fn record_fine_laplacian(a: &CsMat<f64>) {
    #[cfg(feature = "memory-story")]
    imp::with_story(|s| {
        s.laplacian_nodes = a.rows();
        s.laplacian_nnz = a.nnz();
        s.laplacian_bytes = csmat_bytes(a);
        s.operator_repr = "explicit".to_string();
    });
}

/// Record a matrix-free (stencil) fine Laplacian: nothing is stored, so
/// `laplacian_bytes` is 0 and `nnz` is the logical 5-point count.
#[inline]
#[allow(unused_variables)]
pub fn record_stencil_fine(nodes: usize) {
    #[cfg(feature = "memory-story")]
    imp::with_story(|s| {
        s.laplacian_nodes = nodes;
        s.laplacian_nnz = 5 * nodes;
        s.laplacian_bytes = 0;
        s.operator_repr = "stencil".to_string();
    });
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
    #[cfg(feature = "memory-story")]
    imp::with_story(|s| {
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

// ---- coarsest direct solve ----

#[inline]
#[allow(unused_variables)]
pub fn record_coarse_dense(n: usize) {
    #[cfg(feature = "memory-story")]
    imp::with_story(|s| s.coarse_dense_bytes = dense_bytes(n));
}

// ---- CG vectors ----

#[inline]
#[allow(unused_variables)]
pub fn record_cg_vectors(n: usize) {
    #[cfg(feature = "memory-story")]
    imp::with_story(|s| {
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
    #[cfg(feature = "memory-story")]
    imp::with_story(|s| s.jacobi_diag_bytes = vec_f64_bytes(n));
}

// ---- cache ----

#[inline]
#[allow(unused_variables)]
pub fn record_cache_laplacian(a: &CsMat<f64>) {
    #[cfg(feature = "memory-story")]
    imp::with_story(|s| s.cache_laplacian_bytes = csmat_bytes(a));
}

#[inline]
#[allow(unused_variables)]
pub fn record_cache_cell_to_node(n_cells: usize) {
    #[cfg(feature = "memory-story")]
    imp::with_story(|s| s.cache_cell_to_node_bytes = vec_i32_bytes(n_cells));
}

#[inline]
#[allow(unused_variables)]
pub fn record_cache_last_voltages(n: usize) {
    #[cfg(feature = "memory-story")]
    imp::with_story(|s| s.cache_last_voltages_bytes = vec_f64_bytes(n));
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
    fn test_take_story_none_by_default() {
        // A fresh thread (no records) should return None.
        reset();
        assert!(take_story().is_none());
    }

    #[cfg(feature = "memory-story")]
    #[test]
    fn test_record_accumulates() {
        reset();
        record_conductance_grid(100);
        record_cell_to_node_map(100);
        let a = small_csmat();
        record_fine_laplacian(&a);

        let story = take_story().expect("feature enabled -> story present");
        assert_eq!(story.conductance_grid_bytes, vec_f64_bytes(100));
        assert_eq!(story.cell_to_node_map_bytes, vec_i32_bytes(100));
        assert_eq!(story.laplacian_nodes, 3);
        assert_eq!(story.laplacian_nnz, 5);
        assert_eq!(story.laplacian_bytes, csmat_bytes(&a));

        // take() clears the story
        assert!(take_story().is_none());
    }

    #[cfg(feature = "memory-story")]
    #[test]
    fn test_record_level_workspace_sizing() {
        reset();
        record_level(1, 64, 9, 1000, 500, 0, GalerkinScratch::default());
        let story = take_story().unwrap();
        let lvl = &story.hierarchy[0];
        assert_eq!(lvl.level, 1);
        assert_eq!(lvl.nodes, 64);
        assert_eq!(lvl.workspace_e_bytes, vec_f64_bytes(64));
        assert_eq!(lvl.workspace_d_bytes, vec_f64_bytes(64));
        assert_eq!(lvl.workspace_d_prime_bytes, vec_f64_bytes(64));
    }

    #[cfg(feature = "memory-story")]
    #[test]
    fn test_record_stencil_fine_zero_bytes() {
        reset();
        record_stencil_fine(100);
        let story = take_story().unwrap();
        assert_eq!(story.operator_repr, "stencil");
        assert_eq!(story.laplacian_nodes, 100);
        assert_eq!(story.laplacian_nnz, 5 * 100);
        assert_eq!(story.laplacian_bytes, 0);
    }

    #[cfg(feature = "memory-story")]
    #[test]
    fn test_record_fine_laplacian_explicit_repr() {
        reset();
        let a = small_csmat();
        record_fine_laplacian(&a);
        let story = take_story().unwrap();
        assert_eq!(story.operator_repr, "explicit");
    }
}
