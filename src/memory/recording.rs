use super::{GalerkinScratch, InstrumentationProfile};
#[cfg(feature = "instrumentation-profile")]
use super::{sizes::*, LevelMemory};
use crate::linalg::real::Real;
use sprs::CsMat;

#[cfg(feature = "instrumentation-profile")]
mod imp {
    use super::*;
    use std::cell::RefCell;

    // Each thread owns an independent profile so concurrent solves do not
    // overwrite one another's instrumentation.
    thread_local! {
        static PROFILE: RefCell<Option<InstrumentationProfile>> = RefCell::new(None);
    }

    // Closure-based access keeps the mutable RefCell borrow within this update.
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
        PROFILE.with(|slot| slot.borrow_mut().take().map(|mut profile| {
            profile.complete_accounting();
            profile
        }))
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

// Assembly

#[inline]
#[allow(unused_variables)]
pub fn record_conductance_grid(n_cells: usize) { record_conductance_grid_typed::<f64>(n_cells) }

#[inline]
#[allow(unused_variables)]
pub fn record_conductance_grid_typed<F: Real>(n_cells: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| s.conductance_grid_bytes = vec_scalar_bytes::<F>(n_cells));
}

#[inline]
#[allow(unused_variables)]
pub fn record_cell_to_node_map(n_cells: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| s.cell_to_node_map_bytes = vec_i32_bytes(n_cells));
}

#[inline]
#[allow(unused_variables)]
pub fn record_edge_triplets(n_triplets: usize) { record_edge_triplets_typed::<f64>(n_triplets) }

#[inline]
#[allow(unused_variables)]
pub fn record_edge_triplets_typed<F: Real>(n_triplets: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| s.edge_triplets_bytes = triplets_bytes_typed::<F>(n_triplets));
}

// Fine Laplacian

#[inline]
#[allow(unused_variables)]
pub fn record_fine_laplacian<F: Real>(laplacian: &CsMat<F>) {
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
pub fn record_stencil_fine<F: Real>(operator: &crate::linalg::operator::StencilOperator<'_, F>) {
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
pub fn record_fine_operator<F: Real>(operator: &crate::linalg::operator::FineOperator<'_, F>) {
    #[cfg(feature = "instrumentation-profile")]
    match operator {
        crate::linalg::operator::FineOperator::Explicit(matrix) => record_fine_laplacian(matrix),
        crate::linalg::operator::FineOperator::Stencil(stencil) => record_stencil_fine(stencil),
    }
}

#[inline]
#[allow(unused_variables)]
pub fn record_assembly_scratch(nodes: usize, entries: usize) { record_assembly_scratch_typed::<f64>(nodes, entries) }

#[inline]
#[allow(unused_variables)]
pub fn record_assembly_scratch_typed<F: Real>(nodes: usize, entries: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| {
        profile.assembly_row_sums_bytes = vec_scalar_bytes::<F>(nodes);
        profile.assembly_laplacian_triplets_bytes = triplets_bytes_typed::<F>(entries);
    });
}

#[inline]
#[allow(unused_variables)]
pub fn record_assembly_matrix<F: Real>(matrix: &CsMat<F>) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| {
        profile.assembly_matrix_nodes = matrix.rows();
        profile.assembly_matrix_nnz = matrix.nnz();
        profile.assembly_matrix_bytes = csmat_bytes(matrix);
    });
}

#[inline]
#[allow(unused_variables)]
pub fn record_filled_resistance(cells: usize) { record_filled_resistance_typed::<f64>(cells) }

#[inline]
#[allow(unused_variables)]
pub fn record_filled_resistance_typed<F: Real>(cells: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| profile.filled_resistance_bytes = vec_scalar_bytes::<F>(cells));
}

#[inline]
#[allow(unused_variables)]
pub fn record_ground_setup(bytes: u64) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| profile.ground_setup_bytes = bytes);
}

#[inline]
#[allow(unused_variables)]
pub fn record_inputs(cells: usize, rhs_reuses_source_input: bool) { record_inputs_typed::<f64>(cells, rhs_reuses_source_input) }

#[inline]
#[allow(unused_variables)]
pub fn record_inputs_typed<F: Real>(cells: usize, rhs_reuses_source_input: bool) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| {
        profile.scalar_bytes = std::mem::size_of::<F>() as u64;
        profile.precision = if profile.scalar_bytes == 4 { "f32" } else { "f64" }.into();
        profile.input_resistance_bytes = vec_scalar_bytes::<F>(cells);
        profile.input_source_bytes = vec_scalar_bytes::<F>(cells);
        profile.input_ground_bytes = vec_scalar_bytes::<F>(cells);
        profile.rhs_reuses_source_input = rhs_reuses_source_input;
    });
}

#[inline]
pub fn record_source_copy() {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| profile.rhs_reuses_source_input = false);
}

// Hierarchy

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
) { record_level_typed::<f64>(level, nodes, nnz, laplacian_bytes, prolongation_p_bytes, cholesky_factor_bytes, galerkin_scratch) }

#[inline]
#[allow(unused_variables)]
pub fn record_level_typed<F: Real>(
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
        let workspace = vec_scalar_bytes::<F>(nodes);
        s.hierarchy.push(LevelMemory {
            level,
            nodes,
            nnz,
            operator_repr: "explicit".into(),
            laplacian_bytes,
            matrix_values_bytes: if laplacian_bytes > 0 { vec_scalar_bytes::<F>(nnz) } else { 0 },
            matrix_indices_bytes: if laplacian_bytes > 0 { vec_usize_bytes(nnz) } else { 0 },
            matrix_indptr_bytes: if laplacian_bytes > 0 { vec_usize_bytes(nodes + 1) } else { 0 },
            prolongation_p_bytes,
            cholesky_factor_bytes,
            workspace_e_bytes: workspace,
            workspace_d_bytes: workspace,
            workspace_d_prime_bytes: workspace,
            galerkin_scratch,
            ..LevelMemory::default()
        });
    });
}

/// Include fine-grid scratch vectors even when its matrix is a stencil.
#[inline]
#[allow(unused_variables)]
pub fn record_fine_level(cholesky_factor_bytes: u64) { record_fine_level_typed::<f64>(cholesky_factor_bytes) }

#[inline]
#[allow(unused_variables)]
pub fn record_fine_level_typed<F: Real>(cholesky_factor_bytes: u64) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| {
        let workspace = vec_scalar_bytes::<F>(profile.laplacian_nodes);
        profile.hierarchy.push(LevelMemory {
            level: 0,
            nodes: profile.laplacian_nodes,
            nnz: profile.laplacian_nnz,
            operator_repr: profile.operator_repr.clone(),
            laplacian_bytes: profile.laplacian_bytes,
            matrix_values_bytes: if profile.laplacian_bytes > 0 { vec_scalar_bytes::<F>(profile.laplacian_nnz) } else { 0 },
            matrix_indices_bytes: if profile.laplacian_bytes > 0 { vec_usize_bytes(profile.laplacian_nnz) } else { 0 },
            matrix_indptr_bytes: if profile.laplacian_bytes > 0 { vec_usize_bytes(profile.laplacian_nodes + 1) } else { 0 },
            cholesky_factor_bytes,
            // The finest correction and RHS borrow CG's existing vectors.
            workspace_e_bytes: 0,
            workspace_d_bytes: 0,
            workspace_d_prime_bytes: workspace,
            ..LevelMemory::default()
        });
    });
}

/// Set the representation of a previously recorded hierarchy level.
#[inline]
#[allow(unused_variables)]
pub fn set_level_operator_representation(level: usize, representation: &str) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| {
        if let Some(entry) = profile
            .hierarchy
            .iter_mut()
            .find(|entry| entry.level == level)
        {
            entry.operator_repr = representation.into();
        }
    });
}

/// Set persistent smoother and matrix-free operator buffer details separately
/// from the temporary setup allocations in GalerkinScratch.
#[inline]
#[allow(unused_variables)]
pub fn set_level_smoother_and_operator_scratch(
    level: usize,
    smoother: &str,
    diag_bytes: u64,
    omega: Option<f64>,
    matvec_bytes: u64,
) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| {
        if let Some(entry) = profile
            .hierarchy
            .iter_mut()
            .find(|entry| entry.level == level)
        {
            entry.smoother_repr = smoother.into();
            entry.smoother_diag_bytes = diag_bytes;
            entry.smoother_omega = omega;
            entry.operator_matvec_scratch_bytes = matvec_bytes;
            entry.operator_prolonged_bytes = matvec_bytes / 2;
            entry.operator_applied_bytes = matvec_bytes / 2;
        }
    });
}

// Coarsest direct solve

#[inline]
#[allow(unused_variables)]
pub fn record_coarsest(nodes: usize, requested: Option<usize>, cholesky_available: bool) { record_coarsest_typed::<f64>(nodes, requested, cholesky_available) }

#[inline]
#[allow(unused_variables)]
pub fn record_coarsest_typed<F: Real>(nodes: usize, requested: Option<usize>, cholesky_available: bool) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| {
        profile.coarsest_nodes = nodes;
        profile.requested_cholesky_size = requested;
        profile.cholesky_available = cholesky_available;
        if cholesky_available {
            profile.cholesky_solve_scratch_bytes = 2 * vec_scalar_bytes::<F>(nodes);
        } else {
            profile.coarse_fallback_scratch_bytes = 6 * vec_scalar_bytes::<F>(nodes);
        }
    });
}

#[inline]
#[allow(unused_variables)]
pub fn record_coarse_dense(n: usize) { record_coarse_dense_typed::<f64>(n) }

#[inline]
#[allow(unused_variables)]
pub fn record_coarse_dense_typed<F: Real>(n: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| s.coarse_dense_bytes = dense_bytes_typed::<F>(n));
}

// CG vectors

#[inline]
#[allow(unused_variables)]
pub fn record_cg_vectors(n: usize) { record_cg_vectors_typed::<f64>(n) }

pub fn record_mixed_cg_vectors(n: usize) {
    record_cg_vectors_typed::<f32>(n);
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| s.voltage_v_bytes = vec_scalar_bytes::<f64>(n));
}

#[inline]
#[allow(unused_variables)]
pub fn record_cg_vectors_typed<F: Real>(n: usize) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|s| {
        let bytes = vec_scalar_bytes::<F>(n);
        s.voltage_v_bytes = bytes;
        s.residual_r_bytes = bytes;
        s.preconditioned_residual_e0_bytes = bytes;
        s.search_direction_p_bytes = bytes;
        s.matvec_lp_bytes = bytes;
        s.source_s_bytes = bytes;
    });
}
