use sprs::CsMat;

pub fn compute_node_current_map(laplacian: &CsMat<f64>, voltages: &[f64]) -> Vec<f64> {
    let n = laplacian.rows();
    let mut node_currents = vec![0.0f64; n];

    for node in 0..n {
        let mut pos_sum = 0.0f64;
        let mut neg_sum = 0.0f64;
        let vn = voltages[node];
        if let Some(rv) = laplacian.outer_view(node) {
            for (neighbor, &val) in rv.iter() {
                if neighbor == node {
                    continue;
                }
                let conductance = val.abs();
                let dv = vn - voltages[neighbor];
                let branch_current = conductance * dv;

                if branch_current > 0.0 {
                    pos_sum += branch_current;
                } else if branch_current < 0.0 {
                    neg_sum -= branch_current;
                }
            }
        }

        node_currents[node] = pos_sum.max(neg_sum);
    }

    node_currents
}

pub fn reconstruct_grid_map(
    node_values: &[f64],
    nodemap: &[i32],
    n: usize,
) -> Vec<f64> {
    let mut grid = vec![0.0f64; n];
    for aj in 0..n {
        let node = nodemap[aj];
        if node > 0 {
            grid[aj] = node_values[(node - 1) as usize];
        }
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconstruct_grid_map() {
        let nodemap = vec![0, 1, 2, 0, 0, 3];
        let node_values = vec![10.0, 20.0, 30.0];
        let grid = reconstruct_grid_map(&node_values, &nodemap, 2 * 3);
        assert_eq!(grid, vec![0.0, 10.0, 20.0, 0.0, 0.0, 30.0]);
    }
}
