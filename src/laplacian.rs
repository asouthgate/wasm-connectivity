use sprs::{CsMat, CsMatView};
use crate::graph::EdgeTriplets;

pub fn build_laplacian(edges: &EdgeTriplets, num_nodes: usize) -> CsMat<f64> {
    let mut row_sums = vec![0.0f64; num_nodes];

    for k in 0..edges.len() {
        let i = edges.row_indices[k];
        let j = edges.col_indices[k];
        if i != j {
            row_sums[i] += edges.values[k];
        }
    }

    let nnz = edges.len() + num_nodes;
    let mut lap_rows: Vec<usize> = Vec::with_capacity(nnz);
    let mut lap_cols: Vec<usize> = Vec::with_capacity(nnz);
    let mut lap_vals: Vec<f64> = Vec::with_capacity(nnz);

    for k in 0..edges.len() {
        let i = edges.row_indices[k];
        let j = edges.col_indices[k];
        if i != j {
            lap_rows.push(i);
            lap_cols.push(j);
            lap_vals.push(-edges.values[k]);
        }
    }

    for i in 0..num_nodes {
        lap_rows.push(i);
        lap_cols.push(i);
        lap_vals.push(row_sums[i]);
    }

    let tri = sprs::TriMat::from_triplets(
        (num_nodes, num_nodes),
        lap_rows,
        lap_cols,
        lap_vals,
    );
    let mut lap = tri.to_csr();

    let norm: f64 = lap.data().iter().map(|&v| v * v).sum::<f64>().sqrt();
    let epsilon = f64::EPSILON * norm;
    if epsilon > 0.0 {
        for v in lap.data_mut() {
            *v += epsilon;
        }
    }

    lap
}

pub fn get_row_neighbors(lap: &CsMat<f64>, row: usize) -> Vec<(usize, f64)> {
    let mut neighbors = Vec::new();
    if let Some(rv) = lap.outer_view(row) {
        for (col, &val) in rv.iter() {
            if col != row && val != 0.0 {
                neighbors.push((col, val.abs()));
            }
        }
    }
    neighbors
}

pub fn get_adjacency_view(lap: &CsMat<f64>) -> CsMatView<'_, f64> {
    lap.view()
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

        let neighbors = get_row_neighbors(&lap, 0);
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].0, 1);
        assert!((neighbors[0].1 - 5.0).abs() < 1e-10);
    }
}
