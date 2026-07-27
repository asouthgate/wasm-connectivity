use sprs::CsMat;
use crate::graph::EdgeTriplets;

pub fn extract_diag_inv(lap: &CsMat<f64>) -> Vec<f64> {
    let n = lap.rows();
    let mut diag_inv = vec![0.0f64; n];
    for row in 0..n {
        if let Some(rv) = lap.outer_view(row) {
            for (col, &val) in rv.iter() {
                if col == row {
                    let abs_val = val.abs();
                    diag_inv[row] = if abs_val > 1e-15 { 1.0 / abs_val } else { 0.0 };
                    break;
                }
            }
        }
    }
    diag_inv
}

// Regularizes the Laplacian matrix by adding a small value to the diagonal elements,
// in order to ensure numerical stability and avoid singular matrices.
pub fn regularize_laplacian(lap: &mut CsMat<f64>) {
    let norm: f64 = lap.data().iter().map(|&v| v * v).sum::<f64>().sqrt();
    if !norm.is_finite() || norm <= 0.0 {
        return;
    }
    
    // Explicitly ground the first node on every level.
    // This creates a reliable, invariant potential reference across the entire hierarchy.
    if let Some(mut row_vec) = lap.outer_view_mut(0) {
        for (col, val) in row_vec.iter_mut() {
            if col == 0 {
                *val += 1e-5 * norm;
                break;
            }
        }
    }
}
// Compute the effective conductances between nodes
//
// This is the matrix L = D - A
//
// # Arguments
// * `edges` - A reference to the EdgeTriplets containing the edges and their conductance values.
// * `num_nodes` - The total number of nodes in the graph.
// # Returns
// A sparse matrix representing the Laplacian of the graph.
pub fn build_laplacian(edges: &EdgeTriplets, num_nodes: usize) -> CsMat<f64> {
    let mut row_sums = vec![0.0f64; num_nodes];

    let nnz = edges.len() + num_nodes;
    let mut lap_rows: Vec<usize> = Vec::with_capacity(nnz);
    let mut lap_cols: Vec<usize> = Vec::with_capacity(nnz);
    let mut lap_vals: Vec<f64> = Vec::with_capacity(nnz);

    for k in 0..edges.len() {
        let i = edges.row_indices[k];
        row_sums[i] += edges.values[k];

        let j = edges.col_indices[k];
        lap_rows.push(i);
        lap_cols.push(j);
        lap_vals.push(-edges.values[k]);
    }

    for (i, row_sum) in row_sums.iter().enumerate() {
        lap_rows.push(i);
        lap_cols.push(i);
        lap_vals.push(*row_sum);
    }

    let tri = sprs::TriMat::from_triplets(
        (num_nodes, num_nodes),
        lap_rows,
        lap_cols,
        lap_vals,
    );
    let mut lap = tri.to_csr();

    regularize_laplacian(&mut lap);
    lap
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::EdgeTriplets;

    #[test]
    fn test_build_laplacian_two_node() {
        let mut edges = EdgeTriplets::new();
        edges.push(0, 1, 5.0);
        edges.push(1, 0, 5.0);
        let lap = build_laplacian(&edges, 2);
        assert_eq!(lap.rows(), 2);
        assert_eq!(lap.cols(), 2);

        let mut neighbors = Vec::new();
        if let Some(rv) = lap.outer_view(0) {
            for (col, &val) in rv.iter() {
                if col != 0 {
                    neighbors.push((col, val.abs()));
                }
            }
        }
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].0, 1);
        assert!((neighbors[0].1 - 5.0).abs() < 1e-10);
    }
}
