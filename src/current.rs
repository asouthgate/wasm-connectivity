use sprs::CsMat;

/// Compute the current map for each node in the graph given the Laplacian matrix and the node voltages.
///
/// # Arguments
/// * `laplacian` - A reference to the Laplacian matrix of the graph.
/// * `voltages` - A slice containing the voltage values for each node.
/// * `out` - A mutable reference to a vector where the computed current values will be stored.
/// # Panics
/// This function will panic if the length of `voltages` does not match the number of rows in the `laplacian` matrix.
pub fn compute_node_current_map_into(
    laplacian: &CsMat<f64>,
    voltages: &[f64],
    out: &mut Vec<f64>,
) {
    let n = laplacian.rows();
    out.clear();
    out.resize(n, 0.0);

    for node in 0..n {
        let mut pos_sum = 0.0f64;
        let mut neg_sum = 0.0f64;
        let vn = voltages[node];
        let rv = laplacian.outer_view(node).unwrap(); // panic if out of bounds, should not happen
        for (neighbor, &val) in rv.iter() {
            if neighbor == node {
                continue;
            }
            // conductance is positive, and the laplacian off-diagonals
            // should be negative conductances. We can take absolute value
            // or negative.
            debug_assert!(val <= 0.0);
            let conductance = -val;
            let dv = vn - voltages[neighbor];
            let branch_current = conductance * dv;
            // KCL means current in and out is balanced
            if branch_current > 0.0 {
                pos_sum += branch_current;
            } else if branch_current < 0.0 {
                neg_sum -= branch_current;
            }
        }
        // KCL does not hold for boundary   
        out[node] = pos_sum.max(neg_sum);
    }
}

// Compute the current map given graph Laplacian and voltages
pub fn compute_node_current_map(laplacian: &CsMat<f64>, voltages: &[f64]) -> Vec<f64> {
    let mut out = Vec::new();
    compute_node_current_map_into(laplacian, voltages, &mut out);
    out
}

// Retrieve dense 2D grid map from sparse node values and cell_to_node
//
// # Arguments
// * `node_values` - A slice containing the values for each node.
// * `cell_to_node` - A slice containing the mapping from grid indices to node indices (1-based).
// * `n` - The total number of grid points (size of the output vector).
// # Returns
// A vector containing the reconstructed grid values
pub fn reconstruct_grid_map(
    node_values: &[f64],
    cell_to_node: &[i32],
    n: usize,
) -> Vec<f64> {
    debug_assert!(n <= cell_to_node.len(), "Requested grid size `n` exceeds cell_to_node length");
    let mut grid = vec![0.0f64; n];
    for aj in 0..n {
        let node = cell_to_node[aj];
        debug_assert!(node >= 0, "Invalid cell_to_node data: node ID cannot be negative (found {})", node);
        if node != 0 {  // node indices are 1-based in cell_to_node
            grid[aj] = node_values[(node - 1) as usize];
        }
    }
    grid
}


#[cfg(test)]
mod tests {
    use super::*;
    use sprs::CsMat;

    #[test]
    fn test_reconstruct_grid_map() {
        let cell_to_node = vec![0, 1, 2, 0, 0, 3];
        let node_values = vec![10.0, 20.0, 30.0];
        let grid = reconstruct_grid_map(&node_values, &cell_to_node, 2 * 3);
        assert_eq!(grid, vec![0.0, 10.0, 20.0, 0.0, 0.0, 30.0]);
    }

    #[test]
    fn test_reconstruct_grid_map_basic_sparse() {
        let cell_to_node = vec![0, 1, 2, 0, 0, 3];
        let node_values = vec![10.0, 20.0, 30.0];
        let grid = reconstruct_grid_map(&node_values, &cell_to_node, 6);
        assert_eq!(grid, vec![0.0, 10.0, 20.0, 0.0, 0.0, 30.0]);
    }

    #[test]
    fn test_reconstruct_grid_map_shuffled_order() {
        // Nodes don't have to be sequential
        let cell_to_node = vec![3, 0, 1, 0, 2];
        let node_values = vec![100.0, 200.0, 300.0]; // Node 1=100, Node 2=200, Node 3=300
        let grid = reconstruct_grid_map(&node_values, &cell_to_node, 5);
        assert_eq!(grid, vec![300.0, 0.0, 100.0, 0.0, 200.0]);
    }

    fn setup_test_laplacian() -> CsMat<f64> {
        let indptr = vec![0, 3, 6, 9];
        let indices = vec![
            0, 1, 2, // Row 0 columns
            0, 1, 2, // Row 1 columns
            0, 1, 2, // Row 2 columns
        ];
        let data = vec![
             1.0, -1.0,  0.0,
            -1.0,  3.0, -2.0,
             0.0, -2.0,  2.0,
        ]; 
        
        CsMat::new((3, 3), indptr, indices, data)
    }

    #[test]
    fn test_compute_current_equilibrium_internal_node() {
        let laplacian = setup_test_laplacian();
        let voltages = vec![10.0, 6.0, 4.0];
        let mut out = Vec::new();

        compute_node_current_map_into(&laplacian, &voltages, &mut out);

        // At internal Node 1, pos_sum (4.0) and neg_sum (4.0) match exactly.
        let epsilon = 1e-12;
        assert!((out[1] - 4.0).abs() < epsilon, "Expected 4.0A at Node 1, got {}", out[1]);
    }

    #[test]
    fn test_compute_current_at_boundaries() {
        let laplacian = setup_test_laplacian();
        let voltages = vec![10.0, 6.0, 4.0];
        let mut out = Vec::new();
        compute_node_current_map_into(&laplacian, &voltages, &mut out);
        let epsilon = 1e-12;
        assert!((out[0] - 4.0).abs() < epsilon, "Source node should register 4.0A");
        assert!((out[2] - 4.0).abs() < epsilon, "Sink node should register 4.0A");
    }

    #[test]
    fn test_compute_current_zero_everywhere_on_equal_voltage() {
        let laplacian = setup_test_laplacian();
        let voltages = vec![5.0, 5.0, 5.0];
        let mut out = Vec::new();
        compute_node_current_map_into(&laplacian, &voltages, &mut out);
        assert_eq!(out, vec![0.0, 0.0, 0.0], "Current mapping must be completely zeroed out");
    }
}