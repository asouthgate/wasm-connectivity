use sprs::CsMat;
use serde::Serialize;
use std::collections::HashSet;
use crate::{components, pcg as solver, current, cache};
use crate::multigrid::{self, MgPreconditioner};

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum GroundMode {
    /// Conductance-to-ground (Neumann): ground cells add conductance to Laplacian diagonal.
    Neumann,
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
                if gv.is_finite() && gv > 0.0 && (gv - nodata).abs() > 1e-10 {
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
/// Also zeros b at ground entries.
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

/// Zero out b entries at ground node positions.
pub(crate) fn zero_ground_rhs(b: &mut [f64], ground_nodes: &[usize]) {
    for &gn in ground_nodes {
        if gn < b.len() {
            b[gn] = 0.0;
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
/// component / pair solves. Exposed for warm-start benchmarks.
#[derive(Serialize)]
pub struct AnnotatedOutput<T: Serialize> {
    pub output: T,
    pub total_iters: usize,
}

// ----------------------------------------------------------------------------
// Raster-source mode
// ----------------------------------------------------------------------------

/// Cached raster-mode solve. Uses the global cache (`cache` module):
///
/// * `rebuild_laplacian = false` — the resistance-derived Laplacian,
///   cell_to_node, and components are reused as-is; only the source/ground
///   rasters are reinterpreted. This is the dominant interactive web-map
///   case and skips the entire `build_circuit_model` pipeline.
/// * `rebuild_laplacian = true` — the circuit model is rebuilt from
///   `resistance_data`, but the previous solve's voltage field (still
///   cached) is fed to the new PCG run as an initial guess. Useful when
///   the user edits the resistance raster itself.
///
/// In both cases, the new voltage field is written back to the cache so
/// the next call can warm-start from it. Returns the output plus the
/// total PCG iteration count (for warm-vs-cold benchmarks).
pub fn solve_raster_cached(
    resistance_data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    source_data: &[f64],
    ground_data: &[f64],
    max_iter: usize,
    tol: f64,
    remove_average: bool,
    rebuild_laplacian: bool,
    ground_mode: GroundMode,
) -> AnnotatedOutput<RasterOutput> {
    let (laplacian, cell_to_node, num_nodes, components, prior_voltages) =
        obtain_circuit(resistance_data, nrows, ncols, nodata, rebuild_laplacian);

    let ground_nodes = collect_ground_nodes(&cell_to_node, ground_data, nrows, ncols, nodata);
    let ground_set: HashSet<usize> = ground_nodes.iter().copied().collect();

    let current_global = build_global_currents(
        &cell_to_node, num_nodes, nrows, ncols, nodata, source_data,
    );
    let prior_slice = prior_voltages
        .as_ref()
        .map(|v| v.as_slice())
        .filter(|v| !v.is_empty());
    let (a, pin_grounds) = match ground_mode {
        GroundMode::Neumann => {
            let g = build_ground_diagonal(&cell_to_node, num_nodes, nrows, ncols, nodata, ground_data);
            let a = if g.iter().any(|&x| x > 0.0) {
                crate::laplacian::add_diagonal(&laplacian, &g)
            } else {
                laplacian
            };
            (a, false)
        }
        GroundMode::Dirichlet => (laplacian, true),
    };
    let (voltages_global, iters) = solve_component_voltages_warm(
        &components, &current_global, &a,
        num_nodes, max_iter, tol, remove_average, prior_slice,
        &ground_set, pin_grounds,
    );

    let out = build_raster_output(
        &voltages_global, resistance_data, ground_data,
        &cell_to_node, nrows, ncols, nodata, ground_mode,
    );

    cache::store_last_voltages(&out.voltages);

    AnnotatedOutput { output: out, total_iters: iters }
}

// ----------------------------------------------------------------------------
// Cache-aware circuit acquisition
// ----------------------------------------------------------------------------

/// Reuse the cached circuit (no rebuild) or build a fresh one. The
/// returned `prior_voltages` is the cache's most recent per-node voltage
/// field — `None` if the cache was empty or the rebuild path discarded
/// the prior solution. The owned laplacian / cell_to_node / components are left
/// in the cache for the next call by this function (the caller does not
/// need to re-store them).
fn obtain_circuit(
    resistance_data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    rebuild: bool,
) -> (CsMat<f64>, Vec<i32>, usize, Vec<Vec<usize>>, Option<Vec<f64>>) {
    let prior = cache::last_voltages();

    if !rebuild {
        if let Some((cached_nrows, cached_ncols, cached_nodata)) = cache::peek_meta() {
            if (cached_nrows, cached_ncols, cached_nodata) == (nrows, ncols, nodata) {
                // We peek-and-store back to leave the cache intact for the
                // subsequent call (which may be a no-rebuild run too).
                let cached = cache::take().expect("peek_meta indicated cache present");
                let prior_out = if prior.is_empty() { None } else { Some(prior) };
                // We keep the circuit in the cache but drop our local
                // reference to it; the caller instead re-stores the
                // fresh laplacian/cell_to_node/components returned below.
                cache::store(
                    cached.laplacian.clone(), cached.cell_to_node.clone(),
                    cached.num_nodes, cached.components.clone(),
                    cached.nrows, cached.ncols, cached.nodata,
                );
                return (
                    cached.laplacian, cached.cell_to_node, cached.num_nodes,
                    cached.components, prior_out,
                );
            }
        }
    }

    // Rebuild path, or no-rebuild path with a stale/missing cache.
    let (cell_to_node, num_nodes, _edges, laplacian, components) =
        crate::build_circuit_model(resistance_data, nrows, ncols, nodata);
    cache::store(
        laplacian.clone(), cell_to_node.clone(), num_nodes, components.clone(),
        nrows, ncols, nodata,
    );
    let prior_out = if rebuild && !prior.is_empty() { Some(prior) } else { None };
    (laplacian, cell_to_node, num_nodes, components, prior_out)
}

// ----------------------------------------------------------------------------
// Shared solve kernel
// ----------------------------------------------------------------------------

/// Build the per-node ground (shunt) diagonal from the ground raster.
/// Each ground cell contributes its value as a conductance to the 0V
/// reference.
fn build_ground_diagonal(
    cell_to_node: &[i32],
    num_nodes: usize,
    nrows: usize,
    ncols: usize,
    nodata: f64,
    ground_data: &[f64],
) -> Vec<f64> {
    let mut conds = vec![0.0f64; num_nodes];
    for row in 0..nrows {
        for col in 0..ncols {
            let idx = row * ncols + col;
            let node = cell_to_node[idx];
            if node > 0 {
                let node_idx = (node - 1) as usize;
                let gv = ground_data[idx];
                if gv.is_finite() && gv > 0.0 && (gv - nodata).abs() > 1e-10 {
                    conds[node_idx] += gv;
                }
            }
        }
    }
    conds
}

fn build_global_currents(
    cell_to_node: &[i32],
    num_nodes: usize,
    nrows: usize,
    ncols: usize,
    nodata: f64,
    source_data: &[f64],
) -> Vec<f64> {
    let mut current_global = vec![0.0f64; num_nodes];
    for row in 0..nrows {
        for col in 0..ncols {
            let idx = row * ncols + col;
            let node = cell_to_node[idx];
            if node > 0 {
                let node_idx = (node - 1) as usize;
                let sv = source_data[idx];
                if sv.is_finite() && sv > 0.0 && (sv - nodata).abs() > 1e-10 {
                    current_global[node_idx] += sv;
                }
            }
        }
    }
    current_global
}

/// Multigrid-preconditioned raster-mode solve.  Fills nodata cells with a
/// high resistance so the domain is a perfect rectangle, builds a
/// geometric multigrid hierarchy, and uses MG V-cycles as the CG
/// preconditioner.  Returns the output plus total PCG iteration count.
///
/// Unlike the Jacobi-preconditioned path, this solves the full-grid system
/// directly (no component extraction) because the MG hierarchy is built on
/// the full rectangular grid and its restriction/prolongation operators
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
    // Fill nodata → every cell is a node → rectangular grid
    let filled = multigrid::fill_nodata(resistance_data, nodata);

    let (cell_to_node, num_nodes, _edges, laplacian, _components) =
        crate::build_circuit_model(&filled, nrows, ncols, nodata);

    let current_global = build_global_currents(
        &cell_to_node, num_nodes, nrows, ncols, nodata, source_data,
    );

    // Declare the system matrix up front, then run the whole algorithm on
    // it. Neumann adds finite ground conductances to the diagonal; Dirichlet
    // pins ground nodes at V=0. The MG hierarchy is built from the same
    // matrix, so Galerkin coarsening propagates ground effects to all levels.
    let grounds_present;
    let (a, mut b) = match ground_mode {
        GroundMode::Neumann => {
            let g = build_ground_diagonal(&cell_to_node, num_nodes, nrows, ncols, nodata, ground_data);
            grounds_present = g.iter().any(|&x| x > 0.0);
            let a = if grounds_present {
                crate::laplacian::add_diagonal(&laplacian, &g)
            } else {
                laplacian
            };
            (a, current_global)
        }
        GroundMode::Dirichlet => {
            let gns = collect_ground_nodes(&cell_to_node, ground_data, nrows, ncols, nodata);
            grounds_present = !gns.is_empty();
            let a = if grounds_present {
                apply_dirichlet_ground_lap(&laplacian, &gns)
            } else {
                laplacian
            };
            let mut b = current_global;
            zero_ground_rhs(&mut b, &gns);
            (a, b)
        }
    };

    let mg = MgPreconditioner::build_from_laplacian(&a, nrows, ncols, 8);

    // Mean removal is only needed for singular (ground-free) systems; with
    // grounds the system is anchored and Julia solves b as-is.
    if remove_average && !grounds_present {
        let sum: f64 = b.iter().sum();
        if sum.abs() > 1e-15 {
            let mean = sum / num_nodes as f64;
            for v in &mut b {
                *v -= mean;
            }
        }
    }
    let res = solver::cg_solve_precond(&a, &b, max_iter, tol, None, &mg);

    let out = build_raster_output(
        &res.x, resistance_data, ground_data,
        &cell_to_node, nrows, ncols, nodata, ground_mode,
    );

    AnnotatedOutput { output: out, total_iters: res.iters }
}

/// Matrix-dependent (Alcouffe) variant of `solve_raster_sources_mg`.
/// Uses operator-induced prolongation weights derived from the fine-grid
/// Laplacian entries instead of fixed bilinear interpolation. This gives
/// better convergence for problems with strongly varying resistance
/// coefficients (e.g. land-use maps with roads, rivers, and buildings).
pub fn solve_raster_sources_mg_alcouffe(
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
    let filled = multigrid::fill_nodata(resistance_data, nodata);

    let (cell_to_node, num_nodes, _edges, laplacian, _components) =
        crate::build_circuit_model(&filled, nrows, ncols, nodata);

    let current_global = build_global_currents(
        &cell_to_node, num_nodes, nrows, ncols, nodata, source_data,
    );

    // Declare the system matrix up front (see solve_raster_sources_mg).
    let grounds_present;
    let (a, mut b, mg_weights) = match ground_mode {
        GroundMode::Neumann => {
            let g = build_ground_diagonal(&cell_to_node, num_nodes, nrows, ncols, nodata, ground_data);
            grounds_present = g.iter().any(|&x| x > 0.0);
            let a = if grounds_present {
                crate::laplacian::add_diagonal(&laplacian, &g)
            } else {
                laplacian
            };
            (a, current_global, None)
        }
        GroundMode::Dirichlet => {
            let gns = collect_ground_nodes(&cell_to_node, ground_data, nrows, ncols, nodata);
            grounds_present = !gns.is_empty();
            // Alcouffe prolongation weights come from the un-pinned Laplacian;
            // identity rows at ground nodes would corrupt them. Keep it only
            // when pinning actually happens (no grounds -> `a` is already the
            // un-pinned Laplacian).
            let (a, weights) = if grounds_present {
                (apply_dirichlet_ground_lap(&laplacian, &gns), Some(laplacian))
            } else {
                (laplacian, None)
            };
            let mut b = current_global;
            zero_ground_rhs(&mut b, &gns);
            (a, b, weights)
        }
    };

    let mg = MgPreconditioner::build_alcouffe_from_laplacian(&a, mg_weights.as_ref(), nrows, ncols, 8);

    if remove_average && !grounds_present {
        let sum: f64 = b.iter().sum();
        if sum.abs() > 1e-15 {
            let mean = sum / num_nodes as f64;
            for v in &mut b {
                *v -= mean;
            }
        }
    }
    let res = solver::cg_solve_precond(&a, &b, max_iter, tol, None, &mg);

    let out = build_raster_output(
        &res.x, resistance_data, ground_data,
        &cell_to_node, nrows, ncols, nodata, ground_mode,
    );

    AnnotatedOutput { output: out, total_iters: res.iters }
}

fn solve_component_voltages_warm(
    components: &[Vec<usize>],
    current_global: &[f64],
    laplacian: &CsMat<f64>,
    num_nodes: usize,
    max_iter: usize,
    tol: f64,
    remove_average: bool,
    prior_voltages: Option<&[f64]>,
    ground_set: &HashSet<usize>,
    pin_grounds: bool,
) -> (Vec<f64>, usize) {
    let mut voltages_global = vec![0.0f64; num_nodes];
    let mut current_local = Vec::with_capacity(num_nodes);
    let mut total_iters = 0usize;

    let has_ground = !ground_set.is_empty();

    for comp in components {
        let total_current: f64 = comp.iter().map(|&gn| current_global[gn].abs()).sum();
        let has_comp_ground = has_ground && comp.iter().any(|gn| ground_set.contains(gn));
        if !total_current.is_finite() || (total_current < 1e-15 && !has_comp_ground) {
            continue;
        }

        let (a_local, _node_to_local) =
            components::build_subgraph_laplacian(laplacian, comp);
        let comp_size = comp.len();

        current_local.clear();
        for &gn in comp {
            current_local.push(current_global[gn]);
        }

        // For Dirichlet ground: build a modified local Laplacian with
        // identity rows at ground local nodes and zero RHS. (Neumann grounds
        // are already declared on the diagonal of `laplacian` itself.)
        let a_local_mod = if pin_grounds && has_comp_ground {
            let mut local_ground_indices = Vec::new();
            for (li, &gn) in comp.iter().enumerate() {
                if ground_set.contains(&gn) {
                    local_ground_indices.push(li);
                }
            }
            zero_ground_rhs(&mut current_local, &local_ground_indices);
            Some(apply_dirichlet_ground_lap(&a_local, &local_ground_indices))
        } else {
            None
        };
        let a_solve = a_local_mod.as_ref().unwrap_or(&a_local);

        // Mean removal is only needed for singular (ground-free) systems.
        let do_remove = remove_average && !has_comp_ground;
        if do_remove {
            let sum: f64 = current_local.iter().sum();
            if sum.abs() > 1e-15 {
                let mean = sum / comp_size as f64;
                for v in &mut current_local {
                    *v -= mean;
                }
            }
        }

        let local_seed = prior_voltages.map(|pv| {
            let mut seed = Vec::with_capacity(comp_size);
            for &gn in comp {
                seed.push(if gn < pv.len() { pv[gn] } else { 0.0 });
            }
            seed
        });
        let local_seed_ref = local_seed.as_ref().map(|s| s.as_slice());

        let res = solver::cg_solve(a_solve, &current_local, max_iter, tol, local_seed_ref);
        total_iters += res.iters;
        let voltages_local = res.x;

        for (li, &gn) in comp.iter().enumerate() {
            voltages_global[gn] = voltages_local[li];
        }
    }

    (voltages_global, total_iters)
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
    let current_map = current::compute_current_map_from_raster(
        resistance_data,
        ground_data,
        &voltage_map,
        nrows,
        ncols,
        nodata,
        ground_mode == GroundMode::Neumann,
    );

    RasterOutput {
        voltages: voltage_map,
        current_map,
        nrows,
        ncols,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_raster_inputs(size: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
        let n = size * size;
        let res = vec![1.0f64; n];
        let mut src = vec![0.0f64; n];
        let mut gnd = vec![0.0f64; n];
        for row in 0..size {
            src[row * size] = 1.0;
            gnd[row * size + (size - 1)] = 1.0;
        }
        (res, src, gnd)
    }

    #[test]
    fn test_warm_no_rebuild_matches_cold_currentmap() {
        cache::reset();
        let size = 10;
        let (res, src, gnd) = make_raster_inputs(size);

        // Populate the cache with the original solve.
        let _primed = solve_raster_cached(
            &res, size, size, crate::NODATA_SENTINEL,
            &src, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, false, GroundMode::Neumann,
        );

        // Modify sources only — resistance is unchanged so the no-rebuild
        // path should reuse the cached circuit and produce a result
        // numerically equivalent to the cold solve on the same inputs.
        let mut src2 = src.clone();
        src2[5 * size + 5] = 5.0;

        let warm = solve_raster_cached(
            &res, size, size, crate::NODATA_SENTINEL,
            &src2, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, false, GroundMode::Neumann,
        ).output;

        let cold2 = {
            cache::reset();
            solve_raster_cached(
                &res, size, size, crate::NODATA_SENTINEL,
                &src2, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, false, GroundMode::Neumann,
            ).output
        };

        // The Laplacian is near-singular (constant vector null space), so
        // absolute voltages may differ by a constant shift between cold
        // and warm solves; currents (which depend on voltage DIFFERENCES)
        // are invariant to that shift and must match.
        for i in 0..cold2.current_map.len() {
            assert!((warm.current_map[i] - cold2.current_map[i]).abs() < 1e-4,
                "warm (no-rebuild) current diverged at {}: warm={} cold={}",
                i, warm.current_map[i], cold2.current_map[i]);
        }
        cache::reset();
    }

    #[test]
    fn test_warm_rebuild_matches_cold_currentmap() {
        cache::reset();
        let size = 10;
        let (mut res, src, gnd) = make_raster_inputs(size);

        // Initial solve to populate the cache with a baseline voltage field.
        let _ = solve_raster_cached(
            &res, size, size, crate::NODATA_SENTINEL,
            &src, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, false, GroundMode::Neumann,
        );

        // Small edit to the resistance raster; should rebuild and warm-start.
        res[5 * size + 5] = 2.0;

        let warm = solve_raster_cached(
            &res, size, size, crate::NODATA_SENTINEL,
            &src, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, true, GroundMode::Neumann,
        ).output;

        let cold = {
            cache::reset();
            solve_raster_cached(
                &res, size, size, crate::NODATA_SENTINEL,
                &src, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, false, GroundMode::Neumann,
            ).output
        };

        for i in 0..cold.current_map.len() {
            assert!((warm.current_map[i] - cold.current_map[i]).abs() < 1e-4,
                "warm (rebuild) current diverged at {}: warm={} cold={}",
                i, warm.current_map[i], cold.current_map[i]);
        }
        cache::reset();
    }

    #[test]
    fn test_warm_rebuild_reduces_iterations() {
        cache::reset();
        let size = 20;
        let n = size * size;
        let res_orig = vec![1.0f64; n];
        let mut src = vec![0.0f64; n];
        let mut gnd = vec![0.0f64; n];
        src[0] = 1.0;
        gnd[n - 1] = 1.0;

        // Build baseline voltage field on the original resistance.
        let _ = solve_raster_cached(
            &res_orig, size, size, crate::NODATA_SENTINEL,
            &src, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, false, GroundMode::Neumann,
        );

        // Edit the resistance raster by one cell.
        let mut res_edited = res_orig.clone();
        res_edited[size * size / 2] = 1.5;

        // COLD: drop the cache, then rebuild on the edited resistance
        // without a prior voltage seed.
        cache::reset();
        let cold = solve_raster_cached(
            &res_edited, size, size, crate::NODATA_SENTINEL,
            &src, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, true, GroundMode::Neumann,
        );

        // WARM: rebuild the baseline on the original resistance to re-fill
        // the cache with a near-solution voltage field, then rebuild on
        // the edited resistance with that field as a CG seed.
        cache::reset();
        let _baseline = solve_raster_cached(
            &res_orig, size, size, crate::NODATA_SENTINEL,
            &src, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, false, GroundMode::Neumann,
        );
        let warm = solve_raster_cached(
            &res_edited, size, size, crate::NODATA_SENTINEL,
            &src, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, true, GroundMode::Neumann,
        );

        assert!(
            warm.total_iters <= cold.total_iters,
            "warm-start should not require more PCG iterations; cold={} warm={}",
            cold.total_iters, warm.total_iters
        );
        cache::reset();
    }
}
