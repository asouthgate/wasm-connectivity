use crate::linalg::multigrid::{MgOptions, MgPreconditioner};
use crate::linalg::operator::{FineOperator, GroundSpec, StencilOperator};
use crate::linalg::pcg as solver;
use crate::memory;
use serde::Serialize;
use sprs::CsMat;
use std::collections::HashSet;

pub mod current;

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum GroundMode {
    /// Conductance-to-ground (Shunt): ground cells add conductance to Laplacian diagonal.
    Shunt,
    /// Fixed-voltage (Dirichlet): ground cells are pinned at V=0.
    Dirichlet,
}

/// Collect global node indices where ground_data > 0.
fn collect_ground_nodes(
    cell_to_node: &[i32],
    ground_data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
) -> Vec<usize> {
    let mut nodes = Vec::new();
    for row in 0..nrows {
        for col in 0..ncols {
            let idx = row * ncols + col;
            let node = cell_to_node[idx];
            if node > 0 {
                let gv = ground_data[idx];
                if valid_positive_value(gv, nodata) {
                    nodes.push((node - 1) as usize);
                }
            }
        }
    }
    nodes.sort();
    nodes.dedup();
    nodes
}

/// Apply Dirichlet BC (V=0) at ground nodes.
/// Returns a new Laplacian with ground-node rows replaced by identity
/// and ground-node columns removed from non-ground rows.
/// The caller must also zero the source vector at ground entries.
pub(crate) fn apply_dirichlet_ground_lap(
    laplacian: &CsMat<f64>,
    ground_nodes: &[usize],
) -> CsMat<f64> {
    let n = laplacian.rows();
    let ground_set: HashSet<usize> = ground_nodes.iter().copied().collect();

    let mut rows = Vec::new();
    let mut cols = Vec::new();
    let mut vals = Vec::new();

    for row in 0..n {
        if let Some(rv) = laplacian.outer_view(row) {
            if ground_set.contains(&row) {
                rows.push(row);
                cols.push(row);
                vals.push(1.0);
            } else {
                for (col, &val) in rv.iter() {
                    if !ground_set.contains(&col) {
                        rows.push(row);
                        cols.push(col);
                        vals.push(val);
                    }
                }
            }
        }
    }

    sprs::TriMat::from_triplets((n, n), rows, cols, vals).to_csr()
}

/// Zero out source entries at ground node positions.
pub(crate) fn zero_ground_rhs(s: &mut [f64], ground_nodes: &[usize]) {
    for &gn in ground_nodes {
        if gn < s.len() {
            s[gn] = 0.0;
        }
    }
}

#[derive(Serialize)]
pub struct RasterOutput {
    pub voltages: Vec<f64>,
    pub current_map: Vec<f64>,
    pub nrows: usize,
    pub ncols: usize,
}

/// Solve output annotated with the total PCG iteration count across all
/// component / pair solves.
#[derive(Serialize)]
pub struct AnnotatedOutput<T: Serialize> {
    pub output: T,
    pub total_iters: usize,
}

/// Jacobi-preconditioned raster solve. Rebuilds the circuit for every call
/// and starts PCG from zero. Nodata cells are filled with high resistance
/// so the full rectangular system is solved in one shot.
pub fn solve_raster_sources_jacobi(
    resistance_data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    source_data: &[f64],
    ground_data: &[f64],
    max_iter: usize,
    tol: f64,
    remove_average: bool,
    ground_mode: GroundMode,
) -> AnnotatedOutput<RasterOutput> {
    let (laplacian, s, cell_to_node) = prepare_explicit_system(
        resistance_data, nrows, ncols, nodata, source_data, ground_data,
        remove_average, ground_mode,
    );
    memory::record_jacobi_diag(s.len());
    let res = solver::cg_solve(&laplacian, &s, max_iter, tol);

    let out = build_raster_output(
        &res.v,
        resistance_data,
        ground_data,
        &cell_to_node,
        nrows,
        ncols,
        nodata,
        ground_mode,
    );

    AnnotatedOutput {
        output: out,
        total_iters: res.iters,
    }
}


fn prepare_explicit_system(
    resistance_data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    source_data: &[f64],
    ground_data: &[f64],
    remove_average: bool,
    ground_mode: GroundMode,
) -> (CsMat<f64>, Vec<f64>, Vec<i32>) {
    memory::reset();
    let filled = crate::raster::fill_nodata(resistance_data, nodata);
    memory::record_filled_resistance(filled.len());

    let (cell_to_node, num_nodes, _, laplacian) =
        crate::build_circuit_model(&filled, nrows, ncols, nodata);
    let current_global =
        build_node_values(&cell_to_node, num_nodes, nrows, ncols, nodata, source_data);

    let grounds_present;
    let (mut laplacian, mut s) = match ground_mode {
        GroundMode::Shunt => {
            let g =
                build_node_values(&cell_to_node, num_nodes, nrows, ncols, nodata, ground_data);
            memory::record_ground_setup(memory::vec_f64_bytes(g.len()));
            grounds_present = g.iter().any(|&x| x > 0.0);
            let laplacian = if grounds_present {
                crate::circuit::laplacian::add_diagonal(&laplacian, &g)
            } else {
                laplacian
            };
            (laplacian, current_global)
        }
        GroundMode::Dirichlet => {
            let gns = collect_ground_nodes(&cell_to_node, ground_data, nrows, ncols, nodata);
            memory::record_ground_setup(memory::vec_usize_bytes(gns.len()));
            grounds_present = !gns.is_empty();
            let laplacian = if grounds_present {
                apply_dirichlet_ground_lap(&laplacian, &gns)
            } else {
                laplacian
            };
            let mut s = current_global;
            zero_ground_rhs(&mut s, &gns);
            (laplacian, s)
        }
    };

    if !grounds_present {
        crate::circuit::laplacian::regularize_laplacian(&mut laplacian);
    }

    balance_sources(&mut s, remove_average && !grounds_present);

    memory::record_fine_laplacian(&laplacian);
    memory::record_cg_vectors(num_nodes);

    (laplacian, s, cell_to_node)
}

fn valid_positive_value(value: f64, nodata: f64) -> bool {
    value.is_finite() && value > 0.0 && (value - nodata).abs() > 1e-10
}

fn build_node_values(
    cell_to_node: &[i32],
    num_nodes: usize,
    nrows: usize,
    ncols: usize,
    nodata: f64,
    data: &[f64],
) -> Vec<f64> {
    let mut values = vec![0.0; num_nodes];
    for idx in 0..nrows * ncols {
        let node = cell_to_node[idx];
        if node > 0 && valid_positive_value(data[idx], nodata) {
            values[(node - 1) as usize] += data[idx];
        }
    }
    values
}

/// Balance injection only when requested and there are no physical grounds.
fn balance_sources(s: &mut [f64], remove_average: bool) {
    if remove_average {
        let sum: f64 = s.iter().sum();
        if sum.abs() > 1e-15 {
            let mean = sum / s.len() as f64;
            for value in s {
                *value -= mean;
            }
        }
    }
}

/// Multigrid-preconditioned raster-mode solve.  Fills nodata cells with a
/// high resistance so the domain is a perfect rectangle, builds a
/// geometric multigrid hierarchy, and uses MG V-cycles as the CG
/// preconditioner.  Returns the output plus total PCG iteration count.
///
/// This solves the full-grid system directly (no component extraction) because
/// the MG hierarchy is built on the full rectangular grid and its
/// restriction/prolongation operators
/// assume raster-order indexing.
pub fn solve_raster_sources_mg(
    resistance_data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    source_data: &[f64],
    ground_data: &[f64],
    max_iter: usize,
    tol: f64,
    remove_average: bool,
    ground_mode: GroundMode,
) -> AnnotatedOutput<RasterOutput> {
    solve_raster_sources_mg_with_options(
        resistance_data,
        nrows,
        ncols,
        nodata,
        source_data,
        ground_data,
        max_iter,
        tol,
        remove_average,
        ground_mode,
        MgOptions::default(),
    )
}

/// Native variant accepting multigrid setup controls; browser calls use defaults.
pub fn solve_raster_sources_mg_with_options(
    resistance_data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    source_data: &[f64],
    ground_data: &[f64],
    max_iter: usize,
    tol: f64,
    remove_average: bool,
    ground_mode: GroundMode,
    options: MgOptions,
) -> AnnotatedOutput<RasterOutput> {
    let (laplacian, s, cell_to_node) = prepare_explicit_system(
        resistance_data, nrows, ncols, nodata, source_data, ground_data,
        remove_average, ground_mode,
    );

    let mg = MgPreconditioner::build_with_options(
        FineOperator::Explicit(laplacian),
        nrows,
        ncols,
        options,
    );

    let res = solver::cg_solve_precond(mg.fine_operator(), &s, max_iter, tol, &mg);

    let out = build_raster_output(
        &res.v,
        resistance_data,
        ground_data,
        &cell_to_node,
        nrows,
        ncols,
        nodata,
        ground_mode,
    );

    AnnotatedOutput {
        output: out,
        total_iters: res.iters,
    }
}


/// Per-cell source values or shunt conductances in raster order.
fn build_identity_values(data: &[f64], nodata: f64) -> Vec<f64> {
    data.iter()
        .map(|&value| if valid_positive_value(value, nodata) { value } else { 0.0 })
        .collect()
}

/// Per-node Dirichlet pinned mask (node index == flat cell index).
fn build_dirichlet_mask_identity(ground_data: &[f64], nodata: f64) -> Vec<bool> {
    ground_data
        .iter()
        .map(|&gv| valid_positive_value(gv, nodata))
        .collect()
}

/// Matrix-free (low-memory) MG solve: the retained fine Laplacian is a stencil
/// over the input resistance raster; level 1 reconstructs Galerkin rows on
/// demand during setup and level 2 is the first CSR. Matvecs compose transfers
/// with the fine operator; nonterminal levels use weighted Jacobi smoothing.
/// All transfers are geometric stencils.
/// A terminal matrix-free level can use a temporary dense direct solve.
pub fn solve_raster_sources_mg_stencil(
    resistance_data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    source_data: &[f64],
    ground_data: &[f64],
    max_iter: usize,
    tol: f64,
    remove_average: bool,
    ground_mode: GroundMode,
) -> AnnotatedOutput<RasterOutput> {
    solve_raster_sources_mg_stencil_with_options(
        resistance_data,
        nrows,
        ncols,
        nodata,
        source_data,
        ground_data,
        max_iter,
        tol,
        remove_average,
        ground_mode,
        MgOptions::default(),
    )
}

/// Native variant accepting multigrid setup controls; browser calls use defaults.
pub fn solve_raster_sources_mg_stencil_with_options(
    resistance_data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    source_data: &[f64],
    ground_data: &[f64],
    max_iter: usize,
    tol: f64,
    remove_average: bool,
    ground_mode: GroundMode,
    options: MgOptions,
) -> AnnotatedOutput<RasterOutput> {
    memory::reset();
    let num_nodes = nrows * ncols;

    let source_s = build_identity_values(source_data, nodata);

    let grounds_present;
    let (ground_spec, mut s) = match ground_mode {
        GroundMode::Shunt => {
            let shunt = build_identity_values(ground_data, nodata);
            grounds_present = shunt.iter().any(|&x| x > 0.0);
            (GroundSpec::Shunt(shunt), source_s)
        }
        GroundMode::Dirichlet => {
            let mask = build_dirichlet_mask_identity(ground_data, nodata);
            grounds_present = mask.iter().any(|&x| x);
            let mut s = source_s;
            for i in 0..num_nodes {
                if mask[i] {
                    s[i] = 0.0;
                }
            }
            (GroundSpec::Dirichlet(mask), s)
        }
    };

    balance_sources(&mut s, remove_average && !grounds_present);

    let op = StencilOperator::new(nrows, ncols, nodata, resistance_data, ground_spec);

    memory::record_cg_vectors(num_nodes);

    let mg = MgPreconditioner::build_with_options(FineOperator::Stencil(op), nrows, ncols, options);
    let res = solver::cg_solve_precond(mg.fine_operator(), &s, max_iter, tol, &mg);

    let out = build_raster_output_identity(
        res.v,
        resistance_data,
        ground_data,
        nrows,
        ncols,
        nodata,
        ground_mode,
    );

    AnnotatedOutput {
        output: out,
        total_iters: res.iters,
    }
}

/// Raster output for the full-grid (identity mapping) case: the voltage field
/// is already per-cell, so no node scatter is needed.
fn build_raster_output_identity(
    voltage_map: Vec<f64>,
    resistance_data: &[f64],
    ground_data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    ground_mode: GroundMode,
) -> RasterOutput {
    let current_map = current::compute_current_map_from_raster(
        resistance_data,
        ground_data,
        &voltage_map,
        nrows,
        ncols,
        nodata,
        ground_mode == GroundMode::Shunt,
    );
    RasterOutput {
        voltages: voltage_map,
        current_map,
        nrows,
        ncols,
    }
}

fn build_raster_output(
    voltages_global: &[f64],
    resistance_data: &[f64],
    ground_data: &[f64],
    cell_to_node: &[i32],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    ground_mode: GroundMode,
) -> RasterOutput {
    let voltage_map = current::reconstruct_grid_map(voltages_global, cell_to_node, nrows * ncols);
    build_raster_output_identity(
        voltage_map, resistance_data, ground_data, nrows, ncols, nodata, ground_mode,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grounded_solvers_match_uniform_grid_solution() {
        let (nr, nc) = (8, 10);
        for (r, injection, conductance) in [(0.5, 1.0, 0.25), (2.0, 2.5, 3.0)] {
            let resistance = vec![r; nr * nc];
            let mut source = vec![0.0; nr * nc];
            let mut ground = vec![0.0; nr * nc];
            for row in 0..nr {
                source[row * nc] = injection;
                ground[row * nc + nc - 1] = conductance;
            }
            for mode in [GroundMode::Shunt, GroundMode::Dirichlet] {
                let results = [
                    ("jacobi", solve_raster_sources_jacobi(
                        &resistance, nr, nc, -9999.0, &source, &ground,
                        10000, 1e-11, true, mode,
                    )),
                    ("mg", solve_raster_sources_mg(
                        &resistance, nr, nc, -9999.0, &source, &ground,
                        10000, 1e-11, true, mode,
                    )),
                    ("mg-lowmem", solve_raster_sources_mg_stencil(
                        &resistance, nr, nc, -9999.0, &source, &ground,
                        10000, 1e-11, true, mode,
                    )),
                ];
                // Each row is the same series circuit: I*R per edge and I/G
                // at the shunt, or zero volts at a Dirichlet ground.
                let ground_voltage = if mode == GroundMode::Shunt {
                    injection / conductance
                } else {
                    0.0
                };
                for (solver, result) in results {
                    for row in 0..nr {
                        for col in 0..nc {
                            let i = row * nc + col;
                            let expected = ground_voltage + injection * r * (nc - 1 - col) as f64;
                            assert!(
                                (result.output.voltages[i] - expected).abs() < 1e-7,
                                "{solver} {mode:?} R={r} I={injection} G={conductance} cell=({row},{col})"
                            );
                            assert!((result.output.current_map[i] - injection).abs() < 1e-7);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn stencil_solve_matches_explicit_outputs() {
        for (nrows, ncols) in [(3, 5), (15, 17), (16, 16)] {
            let n = nrows * ncols;
            let nodata = crate::NODATA_SENTINEL;
            let mut resistance: Vec<f64> = (0..n).map(|i| 0.5 + (i % 7) as f64).collect();
            resistance[ncols + 1] = nodata;
            resistance[ncols + 2] = f64::NAN;
            resistance[ncols + 3] = 0.0;
            let mut source = vec![0.0; n];
            let mut ground = vec![0.0; n];
            for row in 0..nrows {
                source[row * ncols] = 1.0;
                ground[row * ncols + ncols - 1] = 2.0;
            }
            ground[0] = 1.0; // Also exercise a pinned regularization node.
            for mode in [GroundMode::Shunt, GroundMode::Dirichlet] {
                for (grounded, remove_average) in [
                    (true, true),
                    (true, false),
                    (false, true),
                    (false, false),
                ] {
                    let grounds = if grounded {
                        ground.clone()
                    } else {
                        vec![0.0; n]
                    };
                    let explicit = solve_raster_sources_mg(
                        &resistance,
                        nrows,
                        ncols,
                        nodata,
                        &source,
                        &grounds,
                        5000,
                        1e-9,
                        remove_average,
                        mode,
                    );
                    let stencil = solve_raster_sources_mg_stencil(
                        &resistance,
                        nrows,
                        ncols,
                        nodata,
                        &source,
                        &grounds,
                        5000,
                        1e-9,
                        remove_average,
                        mode,
                    );
                    let jacobi = solve_raster_sources_jacobi(
                        &resistance, nrows, ncols, nodata, &source, &grounds,
                        5000, 1e-9, remove_average, mode,
                    );
                    assert!(
                        explicit.total_iters < 5000 && stencil.total_iters < 5000
                            && jacobi.total_iters < 5000
                    );
                    let spec = match mode {
                        GroundMode::Shunt => GroundSpec::Shunt(grounds.clone()),
                        GroundMode::Dirichlet => GroundSpec::Dirichlet(
                            grounds.iter().map(|&value| value > 0.0).collect(),
                        ),
                    };
                    let operator = StencilOperator::new(nrows, ncols, nodata, &resistance, spec);
                    let mut rhs = source.clone();
                    if mode == GroundMode::Dirichlet {
                        for (value, &ground) in rhs.iter_mut().zip(&grounds) {
                            if ground > 0.0 {
                                *value = 0.0;
                            }
                        }
                    }
                    if !grounded && remove_average {
                        let mean = rhs.iter().sum::<f64>() / n as f64;
                        for value in &mut rhs {
                            *value -= mean;
                        }
                    }
                    let mut product = vec![0.0; n];
                    for result in [&explicit, &stencil, &jacobi] {
                        crate::linalg::operator::Operator::matvec(
                            &operator,
                            &result.output.voltages,
                            &mut product,
                        );
                        let error = product
                            .iter()
                            .zip(&rhs)
                            .map(|(&value, &source)| (value - source).powi(2))
                            .sum::<f64>()
                            .sqrt();
                        let norm = rhs.iter().map(|value| value * value).sum::<f64>().sqrt();
                        assert!(
                            error / norm < 1e-7,
                            "true relative residual: {}",
                            error / norm
                        );
                    }
                    for (label, left, right) in [
                        (
                            "voltage",
                            &explicit.output.voltages,
                            &stencil.output.voltages,
                        ),
                        (
                            "current",
                            &explicit.output.current_map,
                            &stencil.output.current_map,
                        ),
                        (
                            "jacobi current",
                            &explicit.output.current_map,
                            &jacobi.output.current_map,
                        ),
                    ] {
                        for (&a, &b) in left.iter().zip(right) {
                            assert!(a.is_finite() && b.is_finite());
                            assert!(
                                (a - b).abs() <= 1e-5 * (1.0 + a.abs()),
                                "{nrows}x{ncols} {mode:?} grounded={grounded} {label}: {a} vs {b}"
                            );
                        }
                    }
                }
            }
        }
    }
}
