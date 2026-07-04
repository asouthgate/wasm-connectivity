use sprs::CsMat;
use crate::laplacian::get_row_neighbors;

pub fn compute_node_current_map(
    laplacian: &CsMat<f64>,
    voltages: &[f64],
) -> Vec<f64> {
    let n = laplacian.rows();
    let mut node_currents = vec![0.0f64; n];

    for node in 0..n {
        let mut pos_sum = 0.0f64;
        let mut neg_sum = 0.0f64;

        for (neighbor, conductance) in get_row_neighbors(laplacian, node) {
            let dv = voltages[node] - voltages[neighbor];
            let branch_current = conductance * dv;

            if branch_current > 0.0 {
                pos_sum += branch_current;
            } else if branch_current < 0.0 {
                neg_sum -= branch_current;
            }
        }

        node_currents[node] = pos_sum.max(neg_sum);
    }

    node_currents
}

pub fn reconstruct_grid_map(
    node_values: &[f64],
    nodemap: &[i32],
    nrows: usize,
    ncols: usize,
) -> Vec<f64> {
    let mut grid = vec![0.0f64; nrows * ncols];
    for row in 0..nrows {
        for col in 0..ncols {
            let node = nodemap[row * ncols + col];
            if node > 0 {
                grid[row * ncols + col] = node_values[(node - 1) as usize];
            }
        }
    }
    grid
}
