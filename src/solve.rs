use sprs::CsMat;
use serde::Serialize;
use crate::{components, solver, current, grid, cache};
use crate::ConnectivityOutput;
use crate::multigrid::{self, MgPreconditioner};

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
// Point-source (pairwise) mode
// ----------------------------------------------------------------------------

pub fn compute_point_sources(
    laplacian: &CsMat<f64>,
    components: &[Vec<usize>],
    focal_points: &[(i32, usize)],
    nodemap: &[i32],
    nrows: usize,
    ncols: usize,
    max_iter: usize,
    tol: f64,
) -> ConnectivityOutput {
    compute_point_sources_inner(
        laplacian, components, focal_points, nodemap, nrows, ncols,
        max_iter, tol, None,
    ).0
}

pub fn compute_point_sources_warm(
    laplacian: &CsMat<f64>,
    components: &[Vec<usize>],
    focal_points: &[(i32, usize)],
    nodemap: &[i32],
    nrows: usize,
    ncols: usize,
    max_iter: usize,
    tol: f64,
    prior_voltages: Option<&[f64]>,
) -> AnnotatedOutput<ConnectivityOutput> {
    let (out, iters) = compute_point_sources_inner(
        laplacian, components, focal_points, nodemap, nrows, ncols,
        max_iter, tol, prior_voltages,
    );
    AnnotatedOutput { output: out, total_iters: iters }
}

fn compute_point_sources_inner(
    laplacian: &CsMat<f64>,
    components: &[Vec<usize>],
    focal_points: &[(i32, usize)],
    nodemap: &[i32],
    nrows: usize,
    ncols: usize,
    max_iter: usize,
    tol: f64,
    prior_voltages: Option<&[f64]>,
) -> (ConnectivityOutput, usize) {
    let n = components.iter().map(|c| c.len()).sum::<usize>();
    let num_points = focal_points.len();

    let mut resistance_matrix = vec![vec![f64::NAN; num_points]; num_points];
    for (i, row) in resistance_matrix.iter_mut().enumerate() {
        row[i] = 0.0;
    }

    let mut cumulative_currents = vec![0.0f64; n];

    let mut node_to_comp_id: Vec<usize> = vec![usize::MAX; n];
    for (comp_id, comp) in components.iter().enumerate() {
        for &node in comp {
            node_to_comp_id[node] = comp_id;
        }
    }

    let mut voltages_global = vec![0.0f64; n];
    let mut node_current = Vec::new();
    let mut pair_currents = Vec::new();

    let mut comp_focal: Vec<(usize, usize)> = Vec::new();
    let mut total_iters = 0usize;

    for (comp_id, comp) in components.iter().enumerate() {
        let comp_size = comp.len();

        comp_focal.clear();
        for (idx, (_id, node)) in focal_points.iter().enumerate() {
            if node_to_comp_id[*node] == comp_id {
                comp_focal.push((idx, *node));
            }
        }

        if comp_focal.len() < 2 {
            continue;
        }

        let (a_local, node_to_local) =
            components::build_subgraph_laplacian(laplacian, comp);

        for ii in 0..comp_focal.len() {
            let (src_idx, src_node) = comp_focal[ii];

            for &(dst_idx, dst_node) in &comp_focal[(ii + 1)..] {

                node_current.clear();
                node_current.resize(comp_size, 0.0);
                let li_src = node_to_local[&src_node];
                let li_dst = node_to_local[&dst_node];
                node_current[li_src] = -1.0;
                node_current[li_dst] = 1.0;

                let local_seed = prior_voltages.map(|pv| {
                    let mut seed = Vec::with_capacity(comp_size);
                    for &gn in comp {
                        seed.push(if gn < pv.len() { pv[gn] } else { 0.0 });
                    }
                    seed
                });
                let local_seed_ref = local_seed.as_ref().map(|s| s.as_slice());

                let res = solver::cg_solve(&a_local, &node_current, max_iter, tol, local_seed_ref);
                total_iters += res.iters;
                let voltages_local = res.x;

                let v_src = voltages_local[li_src];
                let v_dst = voltages_local[li_dst];
                let resistance = v_dst - v_src;

                if resistance.is_finite() && resistance > 0.0 {
                    resistance_matrix[src_idx][dst_idx] = resistance;
                    resistance_matrix[dst_idx][src_idx] = resistance;
                }

                for slot in voltages_global.iter_mut().take(n) {
                    *slot = 0.0;
                }
                for (li, &gn) in comp.iter().enumerate() {
                    voltages_global[gn] = voltages_local[li] - v_src;
                }

                current::compute_node_current_map_into(laplacian, &voltages_global, &mut pair_currents);
                for (cum, &pc) in cumulative_currents.iter_mut().zip(pair_currents.iter()).take(n) {
                    *cum += pc;
                }
            }
        }
    }

    let current_map = current::reconstruct_grid_map(&cumulative_currents, nodemap, nrows * ncols);
    let point_ids: Vec<i32> = focal_points.iter().map(|(id, _)| *id).collect();

    (
        ConnectivityOutput {
            resistance_matrix,
            current_map,
            nrows,
            ncols,
            point_ids,
        },
        total_iters,
    )
}

// ----------------------------------------------------------------------------
// Raster-source mode
// ----------------------------------------------------------------------------

/// Cold raster-mode solve. Builds the circuit model from scratch and does
/// not touch the warm-start cache. Bit-for-bit identical behaviour to the
/// pre-warm-start implementation.
pub fn compute_raster_sources(
    resistance_data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    source_data: &[f64],
    ground_data: &[f64],
    max_iter: usize,
    tol: f64,
    remove_average: bool,
) -> RasterOutput {
    let (nodemap, num_nodes, _edges, laplacian, components) =
        crate::build_circuit_model(resistance_data, nrows, ncols, nodata);
    let current_global = build_global_currents(
        &nodemap, num_nodes, nrows, ncols, nodata, source_data, ground_data,
    );
    let (voltages_global, _iters) = solve_component_voltages_warm(
        &components, &current_global, &laplacian,
        num_nodes, max_iter, tol, remove_average, None,
    );
    build_raster_output(&voltages_global, &laplacian, &nodemap, nrows, ncols)
}

/// Cached raster-mode solve. Uses the global cache (`cache` module):
///
/// * `rebuild_laplacian = false` — the resistance-derived Laplacian,
///   nodemap, and components are reused as-is; only the source/ground
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
) -> AnnotatedOutput<RasterOutput> {
    let (laplacian, nodemap, num_nodes, components, prior_voltages) =
        obtain_circuit(resistance_data, nrows, ncols, nodata, rebuild_laplacian);

    let current_global = build_global_currents(
        &nodemap, num_nodes, nrows, ncols, nodata, source_data, ground_data,
    );
    let prior_slice = prior_voltages
        .as_ref()
        .map(|v| v.as_slice())
        .filter(|v| !v.is_empty());
    let (voltages_global, iters) = solve_component_voltages_warm(
        &components, &current_global, &laplacian,
        num_nodes, max_iter, tol, remove_average, prior_slice,
    );

    let out = build_raster_output(&voltages_global, &laplacian, &nodemap, nrows, ncols);

    cache::store_last_voltages(&out.voltages);

    AnnotatedOutput { output: out, total_iters: iters }
}

/// Cached point-mode solve. Same `rebuild_laplacian` contract as
/// `solve_raster_cached`.
pub fn solve_point_cached(
    resistance_data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    point_data: &[i32],
    max_iter: usize,
    tol: f64,
    rebuild_laplacian: bool,
) -> AnnotatedOutput<ConnectivityOutput> {
    let (laplacian, components, nodemap, _num_nodes) =
        obtain_circuit_points(resistance_data, nrows, ncols, nodata, rebuild_laplacian);
    let focal_points = grid::extract_focal_points(point_data, nrows, ncols, &nodemap);

    let prior_voltages = cache::last_voltages();
    let prior_slice = if prior_voltages.is_empty() { None } else { Some(prior_voltages.as_slice()) };

    compute_point_sources_warm(
        &laplacian, &components, &focal_points, &nodemap,
        nrows, ncols, max_iter, tol, prior_slice,
    )
}

// ----------------------------------------------------------------------------
// Cache-aware circuit acquisition
// ----------------------------------------------------------------------------

/// Reuse the cached circuit (no rebuild) or build a fresh one. The
/// returned `prior_voltages` is the cache's most recent per-node voltage
/// field — `None` if the cache was empty or the rebuild path discarded
/// the prior solution. The owned laplacian / nodemap / components are left
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
                // fresh laplacian/nodemap/components returned below.
                cache::store(
                    cached.laplacian.clone(), cached.nodemap.clone(),
                    cached.num_nodes, cached.components.clone(),
                    cached.nrows, cached.ncols, cached.nodata,
                );
                return (
                    cached.laplacian, cached.nodemap, cached.num_nodes,
                    cached.components, prior_out,
                );
            }
        }
    }

    // Rebuild path, or no-rebuild path with a stale/missing cache.
    let (nodemap, num_nodes, _edges, laplacian, components) =
        crate::build_circuit_model(resistance_data, nrows, ncols, nodata);
    cache::store(
        laplacian.clone(), nodemap.clone(), num_nodes, components.clone(),
        nrows, ncols, nodata,
    );
    let prior_out = if rebuild && !prior.is_empty() { Some(prior) } else { None };
    (laplacian, nodemap, num_nodes, components, prior_out)
}

fn obtain_circuit_points(
    resistance_data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    rebuild: bool,
) -> (CsMat<f64>, Vec<Vec<usize>>, Vec<i32>, usize) {
    let (laplacian, nodemap, num_nodes, components, _prior) =
        obtain_circuit(resistance_data, nrows, ncols, nodata, rebuild);
    (laplacian, components, nodemap, num_nodes)
}

// ----------------------------------------------------------------------------
// Shared solve kernel
// ----------------------------------------------------------------------------

fn build_global_currents(
    nodemap: &[i32],
    num_nodes: usize,
    nrows: usize,
    ncols: usize,
    nodata: f64,
    source_data: &[f64],
    ground_data: &[f64],
) -> Vec<f64> {
    let mut current_global = vec![0.0f64; num_nodes];
    for row in 0..nrows {
        for col in 0..ncols {
            let idx = row * ncols + col;
            let node = nodemap[idx];
            if node > 0 {
                let node_idx = (node - 1) as usize;
                let sv = source_data[idx];
                let gv = ground_data[idx];
                if sv.is_finite() && sv > 0.0 && (sv - nodata).abs() > 1e-10 {
                    current_global[node_idx] += sv;
                }
                if gv.is_finite() && gv > 0.0 && (gv - nodata).abs() > 1e-10 {
                    current_global[node_idx] -= gv;
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
) -> AnnotatedOutput<RasterOutput> {
    // Fill nodata → every cell is a node → rectangular grid
    let filled = multigrid::fill_nodata(resistance_data, nodata);

    let (nodemap, num_nodes, _edges, laplacian, _components) =
        crate::build_circuit_model(&filled, nrows, ncols, nodata);

    let current_global = build_global_currents(
        &nodemap, num_nodes, nrows, ncols, nodata, source_data, ground_data,
    );

    // Build MG hierarchy on the filled resistance
    let mg = MgPreconditioner::build(&filled, nrows, ncols, nodata, 8);

    // Solve the full-grid system directly (no subgraph extraction)
    let (voltages_global, iters) = solve_full_grid_precond(
        &current_global, &laplacian,
        num_nodes, max_iter, tol, remove_average,
        &mg,
    );

    let out = build_raster_output(&voltages_global, &laplacian, &nodemap, nrows, ncols);

    AnnotatedOutput { output: out, total_iters: iters }
}

/// Solve the full-grid system `L * v = b` with a preconditioner.
/// Unlike `solve_component_voltages_precond`, this does NOT extract
/// subgraph Laplacians — it operates on the full-grid matrix directly.
fn solve_full_grid_precond(
    current_global: &[f64],
    laplacian: &CsMat<f64>,
    num_nodes: usize,
    max_iter: usize,
    tol: f64,
    remove_average: bool,
    precond: &dyn solver::Preconditioner,
) -> (Vec<f64>, usize) {
    let mut b = current_global.to_vec();

    if remove_average {
        let sum: f64 = b.iter().sum();
        if sum.abs() > 1e-15 {
            let mean = sum / num_nodes as f64;
            for v in &mut b {
                *v -= mean;
            }
        }
    }

    let res = solver::cg_solve_precond(laplacian, &b, max_iter, tol, None, precond);
    (res.x, res.iters)
}

fn solve_component_voltages_precond(
    components: &[Vec<usize>],
    current_global: &[f64],
    laplacian: &CsMat<f64>,
    num_nodes: usize,
    max_iter: usize,
    tol: f64,
    remove_average: bool,
    prior_voltages: Option<&[f64]>,
    precond: &dyn solver::Preconditioner,
) -> (Vec<f64>, usize) {
    let mut voltages_global = vec![0.0f64; num_nodes];
    let mut current_local = Vec::with_capacity(num_nodes);
    let mut total_iters = 0usize;

    for comp in components {
        let total_current: f64 = comp.iter().map(|&gn| current_global[gn].abs()).sum();
        if !total_current.is_finite() || total_current < 1e-15 {
            continue;
        }

        let (a_local, _node_to_local) =
            components::build_subgraph_laplacian(laplacian, comp);
        let comp_size = comp.len();

        current_local.clear();
        for &gn in comp {
            current_local.push(current_global[gn]);
        }

        if remove_average {
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

        let res = solver::cg_solve_precond(&a_local, &current_local, max_iter, tol, local_seed_ref, precond);
        total_iters += res.iters;
        let voltages_local = res.x;

        for (li, &gn) in comp.iter().enumerate() {
            voltages_global[gn] = voltages_local[li];
        }
    }

    (voltages_global, total_iters)
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
) -> (Vec<f64>, usize) {
    let mut voltages_global = vec![0.0f64; num_nodes];
    let mut current_local = Vec::with_capacity(num_nodes);
    let mut total_iters = 0usize;

    for comp in components {
        let total_current: f64 = comp.iter().map(|&gn| current_global[gn].abs()).sum();
        if !total_current.is_finite() || total_current < 1e-15 {
            continue;
        }

        let (a_local, _node_to_local) =
            components::build_subgraph_laplacian(laplacian, comp);
        let comp_size = comp.len();

        current_local.clear();
        for &gn in comp {
            current_local.push(current_global[gn]);
        }

        if remove_average {
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

        let res = solver::cg_solve(&a_local, &current_local, max_iter, tol, local_seed_ref);
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
    laplacian: &CsMat<f64>,
    nodemap: &[i32],
    nrows: usize,
    ncols: usize,
) -> RasterOutput {
    let node_currents = current::compute_node_current_map(laplacian, voltages_global);
    let current_map = current::reconstruct_grid_map(&node_currents, nodemap, nrows * ncols);
    let voltage_map = current::reconstruct_grid_map(voltages_global, nodemap, nrows * ncols);

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
    use crate::grid;

    #[test]
    fn test_pairwise_two_points() {
        let (nodemap, _num_nodes, _edges, lap, comps) = crate::build_circuit_model(&[1.0; 25], 5, 5, crate::NODATA_SENTINEL);
        let point_data = {
            let mut p = vec![0i32; 25];
            p[0] = 1;
            p[24] = 2;
            p
        };
        let points = grid::extract_focal_points(&point_data, 5, 5, &nodemap);
        let result = compute_point_sources(&lap, &comps, &points, &nodemap, 5, 5, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL);

        assert_eq!(result.point_ids.len(), 2);
        assert_eq!(result.resistance_matrix.len(), 2);
        assert!(result.resistance_matrix[0][1] > 0.0);
        assert_eq!(result.resistance_matrix[0][0], 0.0);
    }

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
            &src, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, false,
        );

        // Modify sources only — resistance is unchanged so the no-rebuild
        // path should reuse the cached circuit and produce a result
        // numerically equivalent to the cold solve on the same inputs.
        let mut src2 = src.clone();
        src2[5 * size + 5] = 5.0;

        let warm = solve_raster_cached(
            &res, size, size, crate::NODATA_SENTINEL,
            &src2, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, false,
        ).output;

        let cold2 = compute_raster_sources(
            &res, size, size, crate::NODATA_SENTINEL,
            &src2, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true,
        );

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
            &src, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, false,
        );

        // Small edit to the resistance raster; should rebuild and warm-start.
        res[5 * size + 5] = 2.0;

        let warm = solve_raster_cached(
            &res, size, size, crate::NODATA_SENTINEL,
            &src, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, true,
        ).output;

        let cold = compute_raster_sources(
            &res, size, size, crate::NODATA_SENTINEL,
            &src, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true,
        );

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
            &src, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, false,
        );

        // Edit the resistance raster by one cell.
        let mut res_edited = res_orig.clone();
        res_edited[size * size / 2] = 1.5;

        // COLD: drop the cache, then rebuild on the edited resistance
        // without a prior voltage seed.
        cache::reset();
        let cold = solve_raster_cached(
            &res_edited, size, size, crate::NODATA_SENTINEL,
            &src, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, true,
        );

        // WARM: rebuild the baseline on the original resistance to re-fill
        // the cache with a near-solution voltage field, then rebuild on
        // the edited resistance with that field as a CG seed.
        cache::reset();
        let _baseline = solve_raster_cached(
            &res_orig, size, size, crate::NODATA_SENTINEL,
            &src, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, false,
        );
        let warm = solve_raster_cached(
            &res_edited, size, size, crate::NODATA_SENTINEL,
            &src, &gnd, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, true,
        );

        assert!(
            warm.total_iters <= cold.total_iters,
            "warm-start should not require more PCG iterations; cold={} warm={}",
            cold.total_iters, warm.total_iters
        );
        cache::reset();
    }
}