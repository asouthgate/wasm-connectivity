#[cfg(feature = "instrumentation-profile")]
use super::sizes::{usize_size, vec_usize_bytes};
use serde::Serialize;

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
    pub scalar_bytes: u64,
    pub precision: String,
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
            ("values", "matrix_values", nnz as u64 * if self.scalar_bytes == 0 { 8 } else { self.scalar_bytes }),
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

    pub(super) fn complete_accounting(&mut self) {
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
        self.phase("Hierarchy ready", retained.clone(), false);
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
