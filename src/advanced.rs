use crate::grid;
use crate::graph;
use crate::laplacian;
use crate::components;
use crate::solver;
use crate::current;
use serde::Serialize;

#[derive(Serialize)]
pub struct AdvancedOutput {
    pub voltages: Vec<f64>,
    pub current_map: Vec<f64>,
    pub nrows: usize,
    pub ncols: usize,
}

pub fn cal_advanced(
    resistance_data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    source_data: &[f64],
    ground_data: &[f64],
) -> AdvancedOutput {
    let conductance = grid::Grid::to_conductance(resistance_data, nrows, ncols, nodata);

    let (nodemap, num_nodes) = grid::build_nodemap(&conductance);

    let edges = graph::build_adjacency(&conductance, &nodemap);
    let laplacian = laplacian::build_laplacian(&edges, num_nodes);

    let components = components::find_connected_components(&laplacian, num_nodes);

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

    let mut voltages_global = vec![0.0f64; num_nodes];
    let max_iter = 100_000;
    let tol = 1e-6;

    for comp in &components {
        let comp_size = comp.len();
        let comp_set: std::collections::HashSet<usize> = comp.iter().copied().collect();

        let total_current: f64 = comp.iter().map(|&gn| current_global[gn].abs()).sum();
        if total_current < 1e-15 {
            continue;
        }

        let mut node_to_local = vec![0usize; num_nodes];
        for (li, &gn) in comp.iter().enumerate() {
            node_to_local[gn] = li;
        }

        let mut a_local_triplets = graph::EdgeTriplets::new();
        for &gn in comp {
            let li = node_to_local[gn];
            for (neighbor, conductance) in laplacian::get_row_neighbors(&laplacian, gn) {
                if comp_set.contains(&neighbor) && neighbor > gn {
                    let lj = node_to_local[neighbor];
                    a_local_triplets.push(li, lj, conductance);
                    a_local_triplets.push(lj, li, conductance);
                }
            }
        }

        let a_local = laplacian::build_laplacian(&a_local_triplets, comp_size);

        let mut current_local = vec![0.0f64; comp_size];
        for (li, &gn) in comp.iter().enumerate() {
            current_local[li] = current_global[gn];
        }

        let sum: f64 = current_local.iter().sum();
        if sum.abs() > 1e-15 {
            let mean = sum / comp_size as f64;
            for v in &mut current_local {
                *v -= mean;
            }
        }

        let voltages_local = solver::cg_solve(&a_local, &current_local, max_iter, tol);

        for (li, &gn) in comp.iter().enumerate() {
            voltages_global[gn] = voltages_local[li];
        }
    }

    let node_currents = current::compute_node_current_map(&laplacian, &voltages_global);
    let current_map = current::reconstruct_grid_map(&node_currents, &nodemap, nrows, ncols);
    let voltage_map = current::reconstruct_grid_map(&voltages_global, &nodemap, nrows, ncols);

    AdvancedOutput {
        voltages: voltage_map,
        current_map,
        nrows,
        ncols,
    }
}
