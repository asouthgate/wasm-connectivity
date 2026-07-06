use sprs::CsMat;
use crate::solver::cg_solve;
use crate::current::{compute_node_current_map, reconstruct_grid_map};
use crate::ConnectivityOutput as ComputedResult;

pub fn compute_pairwise(
    laplacian: &CsMat<f64>,
    components: &[Vec<usize>],
    focal_points: &[(i32, usize)],
    nodemap: &[i32],
    nrows: usize,
    ncols: usize,
    max_iter: usize,
    tol: f64,
) -> ComputedResult {
    let n = components.iter().map(|c| c.len()).sum::<usize>();
    let num_points = focal_points.len();

    let mut resistance_matrix = vec![vec![-1.0f64; num_points]; num_points];
    for i in 0..num_points {
        resistance_matrix[i][i] = 0.0;
    }

    let mut cumulative_currents = vec![0.0f64; n];

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

        let (a_local, node_to_local) =
            crate::components::build_subgraph_laplacian(laplacian, comp, n);

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
                    resistance_matrix[src_idx][dst_idx] = resistance;
                    resistance_matrix[dst_idx][src_idx] = resistance;
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

    let current_map = reconstruct_grid_map(&cumulative_currents, nodemap, nrows * ncols);
    let point_ids: Vec<i32> = focal_points.iter().map(|(id, _)| *id).collect();

    ComputedResult {
        resistance_matrix,
        current_map,
        nrows,
        ncols,
        point_ids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid;

    #[test]
    fn test_pairwise_two_points() {
        let (nodemap, _num_nodes, _edges, lap, comps) = crate::build_circuit_model(&vec![1.0; 25], 5, 5, crate::NODATA_SENTINEL);
        let point_data = {
            let mut p = vec![0i32; 25];
            p[0] = 1;
            p[24] = 2;
            p
        };
        let points = grid::extract_focal_points(&point_data, 5, 5, &nodemap);
        let result = compute_pairwise(&lap, &comps, &points, &nodemap, 5, 5, crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL);

        assert_eq!(result.point_ids.len(), 2);
        assert_eq!(result.resistance_matrix.len(), 2);
        assert!(result.resistance_matrix[0][1] > 0.0);
        assert_eq!(result.resistance_matrix[0][0], 0.0);
    }
}
