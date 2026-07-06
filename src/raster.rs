use crate::components;
use crate::solver;
use crate::current;
use serde::Serialize;

#[derive(Serialize)]
pub struct RasterOutput {
    pub voltages: Vec<f64>,
    pub current_map: Vec<f64>,
    pub nrows: usize,
    pub ncols: usize,
}

pub fn compute_raster(
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
    let (nodemap, num_nodes, _edges, laplacian, components) = crate::build_circuit_model(resistance_data, nrows, ncols, nodata);

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

    let mut current_local = Vec::with_capacity(num_nodes);

    for comp in &components {
        let total_current: f64 = comp.iter().map(|&gn| current_global[gn].abs()).sum();
        if total_current < 1e-15 {
            continue;
        }

        let (a_local, _node_to_local) =
            components::build_subgraph_laplacian(&laplacian, comp, num_nodes);
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

    let node_currents = current::compute_node_current_map(&laplacian, &voltages_global);
    let current_map = current::reconstruct_grid_map(&node_currents, &nodemap, nrows * ncols);
    let voltage_map = current::reconstruct_grid_map(&voltages_global, &nodemap, nrows * ncols);

    RasterOutput {
        voltages: voltage_map,
        current_map,
        nrows,
        ncols,
    }
}