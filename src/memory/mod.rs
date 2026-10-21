//! Analytical instrumentation of the connectivity solver's named buffers.

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
    /// Bounded stack accumulator used to reconstruct an implicit Galerkin row.
    /// This is reused per row, not multiplied by the level's node count.
    pub row_accumulator_bytes: u64,
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
    /// `explicit`, `stencil`, or `galerkin-stencil`.
    pub operator_repr: String,
    /// Bytes of this level's Laplacian `L_l`.
    pub laplacian_bytes: u64,
    pub matrix_values_bytes: u64,
    pub matrix_indices_bytes: u64,
    pub matrix_indptr_bytes: u64,
    /// Transfer entry-array bytes; zero for generated geometric transfers.
    pub prolongation_p_bytes: u64,
    /// Bytes of the dense Cholesky factor (coarsest level only).
    pub cholesky_factor_bytes: u64,
    /// Bytes of the workspace vector `e_l`.
    pub workspace_e_bytes: u64,
    /// Bytes of the workspace vector `d_l`.
    pub workspace_d_bytes: u64,
    /// Bytes of the workspace vector `d'_l`.
    pub workspace_d_prime_bytes: u64,
    /// Cached inverse diagonal for weighted Jacobi (nonterminal levels only).
    pub smoother_diag_bytes: u64,
    /// Reported smoother: `weighted-jacobi`, `symmetric-gauss-seidel`, or
    /// `none` when no smoothing is used.
    pub smoother_repr: String,
    /// Effective damping after the Jacobi stability bound, if smoothing is used.
    pub smoother_omega: Option<f64>,
    /// Persistent scratch used by a composed operator application.
    pub operator_matvec_scratch_bytes: u64,
    pub operator_prolonged_bytes: u64,
    pub operator_applied_bytes: u64,
    /// Freed Galerkin scratch used to build this level (empty for level 0).
    pub galerkin_scratch: GalerkinScratch,
}

/// Analytical solver payload profile. Hierarchy level 0 repeats the top-level
/// matrix size; count it only once when calculating totals.
#[derive(Serialize, Clone, Default, Debug)]
pub struct InstrumentationProfile {
    pub pointer_bytes: u64,
    pub accounting_basis: String,
    pub input_resistance_bytes: u64,
    pub input_source_bytes: u64,
    pub input_ground_bytes: u64,
    pub rhs_reuses_source_input: bool,
    pub phases: Vec<MemoryPhase>,
    pub peak_named_payload_bytes: u64,
    /// Assembly buffers (not a concurrent peak)
    /// Conductance grid `C = 1/R` (per cell).
    pub conductance_grid_bytes: u64,
    /// Cell-to-node map (per cell, `i32`).
    pub cell_to_node_map_bytes: u64,
    /// Edge triplets (symmetric conductance pairs).
    pub edge_triplets_bytes: u64,
    pub assembly_row_sums_bytes: u64,
    pub assembly_laplacian_triplets_bytes: u64,
    pub assembly_matrix_nodes: usize,
    pub assembly_matrix_nnz: usize,
    pub assembly_matrix_bytes: u64,
    /// Filled resistance copy used by the explicit solve.
    pub filled_resistance_bytes: u64,
    /// Ground construction buffers in the explicit solve (setup category).
    pub ground_setup_bytes: u64,
    /// Owned stencil ground shunts or mask; excludes the borrowed raster.
    pub ground_storage_bytes: u64,

    /// Fine graph Laplacian L (the system matrix)
    pub laplacian_nodes: usize,
    /// Structural entry count; stencil entries occupy zero stored matrix bytes.
    pub laplacian_nnz: usize,
    pub laplacian_bytes: u64,
    /// How the fine Laplacian is represented: `"explicit"` (CSR) or
    /// `"stencil"` (matrix-free). Empty if not yet recorded.
    pub operator_repr: String,

    /// Multigrid hierarchy (level 0 = fine, then coarser)
    pub hierarchy: Vec<LevelMemory>,

    /// Coarsest-level direct solve
    /// Dense coarsest matrix built before factorisation (freed).
    pub coarse_dense_bytes: u64,
    pub requested_cholesky_size: Option<usize>,
    pub coarsest_nodes: usize,
    pub cholesky_available: bool,
    /// Temporary forward/back substitution outputs per direct solve.
    pub cholesky_solve_scratch_bytes: u64,
    /// CG/Jacobi buffers when direct factorization is unavailable.
    pub coarse_fallback_scratch_bytes: u64,

    /// CG workspace vectors (persistent through the solve)
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
}

#[derive(Serialize, Clone, Debug)]
pub struct MemoryObject {
    pub name: String,
    pub category: String,
    pub level: Option<usize>,
    pub bytes: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct MemoryAlias {
    pub name: String,
    pub allocation: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct MemoryPhase {
    pub name: String,
    pub objects: Vec<MemoryObject>,
    pub aliases: Vec<MemoryAlias>,
    pub total_bytes: u64,
}

#[cfg(feature = "instrumentation-profile")]
fn add_object(objects: &mut Vec<MemoryObject>, name: impl Into<String>, category: &str,
    level: Option<usize>, bytes: u64)
{
    if bytes > 0 {
        objects.push(MemoryObject { name: name.into(), category: category.into(), level, bytes });
    }
}

#[cfg(feature = "instrumentation-profile")]
impl InstrumentationProfile {
    fn inputs(&self) -> Vec<MemoryObject> {
        let mut objects = Vec::new();
        for (name, bytes) in [("input.resistance", self.input_resistance_bytes),
            ("input.source", self.input_source_bytes), ("input.ground", self.input_ground_bytes)]
        {
            add_object(&mut objects, name, "inputs", None, bytes);
        }
        objects
    }

    fn system(&self) -> Vec<MemoryObject> {
        let mut objects = self.inputs();
        if !self.rhs_reuses_source_input {
            add_object(&mut objects, "system.rhs", "rhs", None, self.source_s_bytes);
        }
        add_object(&mut objects, "system.cell_to_node", "mapping", None, self.cell_to_node_map_bytes);
        add_object(&mut objects, "system.ground", "grounds", None, self.ground_storage_bytes);
        self.matrix(&mut objects, 0, self.laplacian_nodes, self.laplacian_nnz, self.laplacian_bytes);
        objects
    }

    fn matrix(&self, objects: &mut Vec<MemoryObject>, level: usize, nodes: usize, nnz: usize, bytes: u64) {
        if bytes == 0 { return; }
        for (part, category, bytes) in [
            ("values", "matrix_values", vec_f64_bytes(nnz)),
            ("indices", "matrix_indices", vec_usize_bytes(nnz)),
            ("indptr", "matrix_indptr", vec_usize_bytes(nodes + 1)),
        ] {
            add_object(objects, format!("L{level}.matrix.{part}"), category, Some(level), bytes);
        }
    }

    fn level_operator(&self, objects: &mut Vec<MemoryObject>, level: &LevelMemory) {
        let i = level.level;
        if i > 0 {
            self.matrix(objects, i, level.nodes, level.nnz, level.laplacian_bytes);
        }
        add_object(objects, format!("L{i}.operator.prolonged"), "implicit_prolonged", Some(i), level.operator_prolonged_bytes);
        add_object(objects, format!("L{i}.operator.applied"), "implicit_applied", Some(i), level.operator_applied_bytes);
        add_object(objects, format!("L{i}.transfer"), "transfers", Some(i), level.prolongation_p_bytes);
    }

    fn level_workspace(&self, objects: &mut Vec<MemoryObject>, level: &LevelMemory) {
        let i = level.level;
        for (name, category, bytes) in [
            ("e", "cycle_correction", level.workspace_e_bytes),
            ("d", "cycle_rhs", level.workspace_d_bytes),
            ("d_prime", "cycle_residual", level.workspace_d_prime_bytes),
            ("diag_inv", "diagonals", level.smoother_diag_bytes),
            ("cholesky", "factor", level.cholesky_factor_bytes),
        ] {
            add_object(objects, format!("L{i}.{name}"), category, Some(i), bytes);
        }
    }

    fn phase(&mut self, name: impl Into<String>, objects: Vec<MemoryObject>, cg_aliases: bool) {
        let name = name.into();
        let mut aliases = Vec::new();
        if self.rhs_reuses_source_input && name != "Inputs" && name != "Returned maps" {
            aliases.push(MemoryAlias { name: "system.rhs".into(), allocation: "input.source".into() });
        }
        if self.operator_repr == "stencil" && self.ground_storage_bytes == 0
            && self.input_ground_bytes > 0 && name != "Inputs" && name != "Returned maps"
        {
            aliases.push(MemoryAlias { name: "L0.ground".into(), allocation: "input.ground".into() });
        }
        if cg_aliases && !self.hierarchy.is_empty() {
            aliases.push(MemoryAlias { name: "L0.e".into(), allocation: "cg.e0".into() });
            aliases.push(MemoryAlias { name: "L0.d".into(), allocation: "cg.r".into() });
        }
        if name == "Output construction" && self.operator_repr == "stencil" {
            aliases.push(MemoryAlias { name: "output.voltages".into(), allocation: "solution.voltage".into() });
        }
        let total_bytes = objects.iter().map(|object| object.bytes).sum();
        self.phases.push(MemoryPhase { name, objects, aliases, total_bytes });
    }

    fn complete_accounting(&mut self) {
        self.pointer_bytes = usize_size();
        self.accounting_basis = "Analytical live named heap payloads for solver arguments and buffers; aliases count once. Excludes caller extras, allocator overhead/capacity, container headers, stack, sparse-library internal conversion buffers, rasterization and JSON serialization. Phase totals are not measured allocator peaks.".into();
        self.phases.clear();
        self.phase("Inputs", self.inputs(), false);
        if self.operator_repr == "explicit" {
            let mut assembly = self.inputs();
            for (name, category, bytes) in [
                ("assembly.filled_resistance", "setup", self.filled_resistance_bytes),
                ("assembly.conductance", "setup", self.conductance_grid_bytes),
                ("assembly.cell_to_node", "mapping", self.cell_to_node_map_bytes),
                ("assembly.edges", "setup", self.edge_triplets_bytes),
                ("assembly.row_sums", "setup", self.assembly_row_sums_bytes),
                ("assembly.triplets", "setup", self.assembly_laplacian_triplets_bytes),
            ] { add_object(&mut assembly, name, category, None, bytes); }
            self.matrix(&mut assembly, 0, self.assembly_matrix_nodes,
                self.assembly_matrix_nnz, self.assembly_matrix_bytes);
            self.phase("Fine matrix assembly", assembly, false);
            let mut grounding = self.system();
            add_object(&mut grounding, "grounding.temporary", "setup", None, self.ground_setup_bytes);
            self.phase("Ground preparation", grounding, false);
        }
        self.phase("System ready", self.system(), false);
        let mut operators = self.system();
        for level in self.hierarchy.clone() {
            self.level_operator(&mut operators, &level);
            if level.level == 0 { continue; }
            let mut building = operators.clone();
            let scratch = &level.galerkin_scratch;
            for (name, bytes) in [
                ("seen", scratch.symbolic_seen_bytes), ("row_nnz", scratch.symbolic_row_nnz_bytes),
                ("pos", scratch.numeric_pos_bytes), ("row_count", scratch.numeric_row_count_bytes),
                ("P_triplets", scratch.p_triplets_clone_bytes), ("P_csr", scratch.p_csr_bytes),
                ("P_csc", scratch.p_csc_bytes),
            ] { add_object(&mut building, format!("L{}.setup.{name}", level.level), "setup", Some(level.level), bytes); }
            self.phase(format!("L{} operator setup", level.level), building, false);
        }
        if self.coarse_dense_bytes > 0 {
            let mut factorizing = operators.clone();
            add_object(&mut factorizing, "coarse.dense_matrix", "setup", None, self.coarse_dense_bytes);
            add_object(&mut factorizing, "coarse.factor_candidate", "factor", None, self.coarse_dense_bytes);
            self.phase("Coarse factorization", factorizing, false);
        }
        let mut retained = operators;
        for level in &self.hierarchy { self.level_workspace(&mut retained, level); }
        add_object(&mut retained, "jacobi.diag_inv", "diagonals", None, self.jacobi_diag_bytes);
        self.phase(if self.hierarchy.is_empty() { "Jacobi ready" } else { "Hierarchy ready" }, retained.clone(), false);
        let mut cg = retained.clone();
        for (name, category, bytes) in [
            ("cg.v", "cg_voltage", self.voltage_v_bytes),
            ("cg.r", "cg_residual", self.residual_r_bytes),
            ("cg.e0", "cg_preconditioned", self.preconditioned_residual_e0_bytes),
            ("cg.p", "cg_direction", self.search_direction_p_bytes),
            ("cg.Lp", "cg_matvec", self.matvec_lp_bytes),
        ] { add_object(&mut cg, name, category, None, bytes); }
        self.phase("CG vectors live", cg.clone(), true);
        if self.cholesky_solve_scratch_bytes + self.coarse_fallback_scratch_bytes > 0 {
            if self.cholesky_solve_scratch_bytes > 0 {
                let each = self.cholesky_solve_scratch_bytes / 2;
                add_object(&mut cg, "coarse.forward_solution", "coarse_solve", None, each);
                add_object(&mut cg, "coarse.backward_solution", "coarse_solve", None, each);
            }
            add_object(&mut cg, "coarse.fallback_CG", "coarse_solve", None, self.coarse_fallback_scratch_bytes);
            self.phase("V-cycle coarse solve", cg, true);
        }
        // The CG work vectors have been freed; only its returned voltage remains.
        retained.retain(|object| object.name != "jacobi.diag_inv");
        let mut final_smoothing = retained.clone();
        add_object(&mut final_smoothing, "solution.voltage", "cg_voltage", None, self.voltage_v_bytes);
        if !self.hierarchy.is_empty() { self.phase("Final smoothing", final_smoothing.clone(), false); }
        let mut output = final_smoothing;
        if self.operator_repr == "explicit" {
            add_object(&mut output, "output.voltages", "outputs", None, self.voltage_v_bytes);
        }
        add_object(&mut output, "output.currents", "outputs", None, self.voltage_v_bytes);
        self.phase("Output construction", output, false);
        let mut returned = self.inputs();
        add_object(&mut returned, "output.voltages", "outputs", None, self.voltage_v_bytes);
        add_object(&mut returned, "output.currents", "outputs", None, self.voltage_v_bytes);
        self.phase("Returned maps", returned, false);
        self.peak_named_payload_bytes = self.phases.iter().map(|phase| phase.total_bytes).max().unwrap_or(0);
    }
}

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

// Fine Laplacian

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
pub fn record_assembly_matrix(matrix: &CsMat<f64>) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| {
        profile.assembly_matrix_nodes = matrix.rows();
        profile.assembly_matrix_nnz = matrix.nnz();
        profile.assembly_matrix_bytes = csmat_bytes(matrix);
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

#[inline]
#[allow(unused_variables)]
pub fn record_inputs(cells: usize, rhs_reuses_source_input: bool) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| {
        profile.input_resistance_bytes = vec_f64_bytes(cells);
        profile.input_source_bytes = vec_f64_bytes(cells);
        profile.input_ground_bytes = vec_f64_bytes(cells);
        profile.rhs_reuses_source_input = rhs_reuses_source_input;
    });
}

#[inline]
pub fn record_source_copy() {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| profile.rhs_reuses_source_input = false);
}

// Hierarcy

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
            operator_repr: "explicit".into(),
            laplacian_bytes,
            matrix_values_bytes: if laplacian_bytes > 0 { vec_f64_bytes(nnz) } else { 0 },
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
pub fn record_fine_level(cholesky_factor_bytes: u64) {
    #[cfg(feature = "instrumentation-profile")]
    imp::with_profile(|profile| {
        let workspace = vec_f64_bytes(profile.laplacian_nodes);
        profile.hierarchy.push(LevelMemory {
            level: 0,
            nodes: profile.laplacian_nodes,
            nnz: profile.laplacian_nnz,
            operator_repr: profile.operator_repr.clone(),
            laplacian_bytes: profile.laplacian_bytes,
            matrix_values_bytes: if profile.laplacian_bytes > 0 { vec_f64_bytes(profile.laplacian_nnz) } else { 0 },
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

// CG vectors

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

    #[cfg(feature = "instrumentation-profile")]
    #[test]
    fn phase_accounting_counts_shared_buffers_once_and_releases_cg_scratch() {
        use crate::linalg::multigrid::MgOptions;
        use crate::solve::{self, GroundMode};
        let n = 32 * 32;
        let resistance = vec![1.0; n];
        let mut source = vec![0.0; n];
        source[0] = 1.0;
        let mut ground = vec![0.0; n];
        ground[n - 1] = 1.0;
        solve::solve_raster_sources_mg_stencil_in_place_with_options(
            &resistance, 32, 32, -9999.0, &mut source, &ground,
            1000, 1e-4, true, GroundMode::Shunt, MgOptions::default(),
        );
        let profile = take_profile().unwrap();
        assert!(profile.rhs_reuses_source_input);
        for phase in &profile.phases {
            let names: std::collections::HashSet<_> = phase.objects.iter().map(|o| &o.name).collect();
            assert_eq!(names.len(), phase.objects.len());
            assert_eq!(phase.total_bytes, phase.objects.iter().map(|o| o.bytes).sum::<u64>());
            for alias in &phase.aliases {
                assert!(!names.contains(&alias.name));
                assert!(names.contains(&alias.allocation));
            }
        }
        let cg = profile.phases.iter().find(|p| p.name == "CG vectors live").unwrap();
        assert!(!cg.objects.iter().any(|o| o.name == "system.rhs" || o.name == "L0.e" || o.name == "L0.d"));
        assert_eq!(cg.objects.iter().filter(|o| o.name.starts_with("cg.")).map(|o| o.bytes).sum::<u64>(), 5 * vec_f64_bytes(n));
        let final_phase = profile.phases.iter().find(|p| p.name == "Final smoothing").unwrap();
        assert_eq!(cg.total_bytes - final_phase.total_bytes, 4 * vec_f64_bytes(n));
        let returned = profile.phases.last().unwrap();
        assert_eq!(returned.total_bytes, 5 * vec_f64_bytes(n));
        for level in &profile.hierarchy {
            assert_eq!(level.laplacian_bytes,
                level.matrix_values_bytes + level.matrix_indices_bytes + level.matrix_indptr_bytes);
            assert_eq!(level.operator_matvec_scratch_bytes,
                level.operator_prolonged_bytes + level.operator_applied_bytes);
        }
        let shared_cg_bytes = cg.total_bytes;
        solve::solve_raster_sources_mg_stencil_with_options(
            &resistance, 32, 32, -9999.0, &source, &ground,
            1000, 1e-4, true, GroundMode::Shunt, MgOptions::default(),
        );
        let copied = take_profile().unwrap();
        assert!(!copied.rhs_reuses_source_input);
        let copied_cg = copied.phases.iter().find(|p| p.name == "CG vectors live").unwrap();
        assert_eq!(copied_cg.total_bytes - shared_cg_bytes, vec_f64_bytes(n));
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
            assert!(
                take_profile().is_none(),
                "disabled recording must be a no-op"
            );
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
        let operator =
            StencilOperator::new(10, 10, -9999.0, &resistance, GroundSpec::Dirichlet(mask));
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
        assert_eq!(profile.hierarchy[0].workspace_e_bytes, 0);
        assert_eq!(profile.hierarchy[0].workspace_d_bytes, 0);
        assert_eq!(profile.hierarchy[0].workspace_d_prime_bytes, 800);
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
