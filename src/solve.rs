use sprs::CsMat;
use serde::Serialize;
use crate::{components, solver, current};
use crate::ConnectivityOutput;

#[derive(Serialize)]
pub struct RasterOutput {
    pub voltages: Vec<f64>,
    pub current_map: Vec<f64>,
    pub nrows: usize,
    pub ncols: usize,
}

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
            components::build_subgraph_laplacian(laplacian, comp, n);

        for ii in 0..comp_focal.len() {
            let (src_idx, src_node) = comp_focal[ii];

            for jj in (ii + 1)..comp_focal.len() {
                let (dst_idx, dst_node) = comp_focal[jj];

                let mut node_current = vec![0.0f64; comp_size];
                let li_src = node_to_local[src_node];
                let li_dst = node_to_local[dst_node];
                node_current[li_src] = -1.0;
                node_current[li_dst] = 1.0;

                let voltages_local = solver::cg_solve(&a_local, &node_current, max_iter, tol);

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

                let pair_currents = current::compute_node_current_map(laplacian, &voltages_global);
                for i_global in 0..n {
                    cumulative_currents[i_global] += pair_currents[i_global];
                }
            }
        }
    }

    let current_map = current::reconstruct_grid_map(&cumulative_currents, nodemap, nrows * ncols);
    let point_ids: Vec<i32> = focal_points.iter().map(|(id, _)| *id).collect();

    ConnectivityOutput {
        resistance_matrix,
        current_map,
        nrows,
        ncols,
        point_ids,
    }
}

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
        &nodemap, num_nodes, nrows, ncols, nodata, source_data, ground_data
    );
    let voltages_global = solve_component_voltages(
        &components, &current_global, &laplacian, num_nodes, max_iter, tol, remove_average
    );
    build_raster_output(&voltages_global, &laplacian, &nodemap, nrows, ncols)
}

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
                if sv > 0.0 && (sv - nodata).abs() > 1e-10 {
                    current_global[node_idx] += sv;
                }
                if gv > 0.0 && (gv - nodata).abs() > 1e-10 {
                    current_global[node_idx] -= gv;
                }
            }
        }
    }
    current_global
}

fn solve_component_voltages(
    components: &[Vec<usize>],
    current_global: &[f64],
    laplacian: &CsMat<f64>,
    num_nodes: usize,
    max_iter: usize,
    tol: f64,
    remove_average: bool,
) -> Vec<f64> {
    let mut voltages_global = vec![0.0f64; num_nodes];
    let mut current_local = Vec::with_capacity(num_nodes);

    for comp in components {
        let total_current: f64 = comp.iter().map(|&gn| current_global[gn].abs()).sum();
        if total_current < 1e-15 {
            continue;
        }

        let (a_local, _node_to_local) =
            components::build_subgraph_laplacian(laplacian, comp, num_nodes);
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

        let voltages_local = solver::cg_solve(&a_local, &current_local, max_iter, tol);

        for (li, &gn) in comp.iter().enumerate() {
            voltages_global[gn] = voltages_local[li];
        }
    }

    voltages_global
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
        let (nodemap, _num_nodes, _edges, lap, comps) = crate::build_circuit_model(&vec![1.0; 25], 5, 5, crate::NODATA_SENTINEL);
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
}
