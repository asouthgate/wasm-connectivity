use sprs::CsMat;
use crate::solver::cg_solve;
use crate::current::{compute_node_current_map, reconstruct_grid_map};

pub struct ComputedResult {
    pub resistances: Vec<Vec<f64>>,
    pub current_map: Vec<f64>,
    pub nrows: usize,
    pub ncols: usize,
    pub point_ids: Vec<i32>,
}

pub fn compute_pairwise(
    laplacian: &CsMat<f64>,
    components: &[Vec<usize>],
    focal_points: &[(i32, usize)],
    nodemap: &[i32],
    nrows: usize,
    ncols: usize,
) -> ComputedResult {
    let n = components.iter().map(|c| c.len()).sum::<usize>();
    let num_points = focal_points.len();

    let mut resistances = vec![vec![-1.0f64; num_points]; num_points];
    for i in 0..num_points {
        resistances[i][i] = 0.0;
    }

    let mut cumulative_currents = vec![0.0f64; n];
    let max_iter = 100_000;
    let tol = 1e-6;

    for comp in components {
        let comp_size = comp.len();
        let comp_set: std::collections::HashSet<usize> = comp.iter().copied().collect();

        let comp_focal: Vec<(usize, usize)> = focal_points
            .iter()
            .enumerate()
            .filter(|(_, (_, node))| comp_set.contains(node))
            .map(|(idx, (_, node))| (idx, *node))
            .collect();

        if comp_focal.len() < 2 {
            continue;
        }

        let mut node_to_local = vec![0usize; n];
        for (li, &gn) in comp.iter().enumerate() {
            node_to_local[gn] = li;
        }

        let mut a_local_triplets = crate::graph::EdgeTriplets::new();
        for &gn in comp {
            let li = node_to_local[gn];
            for (neighbor, conductance) in crate::laplacian::get_row_neighbors(laplacian, gn) {
                if comp_set.contains(&neighbor) && neighbor > gn {
                    let lj = node_to_local[neighbor];
                    a_local_triplets.push(li, lj, conductance);
                    a_local_triplets.push(lj, li, conductance);
                }
            }
        }

        let a_local = crate::laplacian::build_laplacian(&a_local_triplets, comp_size);

        for ii in 0..comp_focal.len() {
            let (src_idx, src_node) = comp_focal[ii];

            for jj in (ii + 1)..comp_focal.len() {
                let (dst_idx, dst_node) = comp_focal[jj];

                let mut current = vec![0.0f64; comp_size];
                let li_src = node_to_local[src_node];
                let li_dst = node_to_local[dst_node];
                current[li_src] = -1.0;
                current[li_dst] = 1.0;

                let voltages_local = cg_solve(&a_local, &current, max_iter, tol);

                let v_src = voltages_local[li_src];
                let v_dst = voltages_local[li_dst];
                let resistance = v_dst - v_src;

                if resistance > 0.0 {
                    resistances[src_idx][dst_idx] = resistance;
                    resistances[dst_idx][src_idx] = resistance;
                }

                let mut voltages_global = vec![0.0f64; n];
                for (li, &gn) in comp.iter().enumerate() {
                    voltages_global[gn] = voltages_local[li] - v_src;
                }

                let pair_currents = compute_node_current_map(laplacian, &voltages_global);
                for i_global in 0..n {
                    cumulative_currents[i_global] += pair_currents[i_global];
                }
            }
        }
    }

    let current_map = reconstruct_grid_map(&cumulative_currents, nodemap, nrows, ncols);
    let point_ids: Vec<i32> = focal_points.iter().map(|(id, _)| *id).collect();

    ComputedResult {
        resistances,
        current_map,
        nrows,
        ncols,
        point_ids,
    }
}
