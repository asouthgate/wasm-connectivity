use sprs::CsMat;
use crate::graph::EdgeTriplets;

pub fn regularize_laplacian(lap: &mut CsMat<f64>) {
    let norm: f64 = lap.data().iter().map(|&v| v * v).sum::<f64>().sqrt();
    let epsilon = f64::EPSILON * norm;
    if epsilon > 0.0 {
        for v in lap.data_mut() {
            *v += epsilon;
        }
    }
}

pub fn build_laplacian(edges: &EdgeTriplets, num_nodes: usize) -> CsMat<f64> {
    let mut row_sums = vec![0.0f64; num_nodes];

    for k in 0..edges.len() {
        let i = edges.row_indices[k];
        row_sums[i] += edges.values[k];
    }

    let nnz = edges.len() + num_nodes;
    let mut lap_rows: Vec<usize> = Vec::with_capacity(nnz);
    let mut lap_cols: Vec<usize> = Vec::with_capacity(nnz);
    let mut lap_vals: Vec<f64> = Vec::with_capacity(nnz);

    for k in 0..edges.len() {
        let i = edges.row_indices[k];
        let j = edges.col_indices[k];
        lap_rows.push(i);
        lap_cols.push(j);
        lap_vals.push(-edges.values[k]);
    }

    for (i, row_sum) in row_sums.iter_mut().enumerate() {
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

pub fn get_row_neighbors(lap: &CsMat<f64>, row: usize) -> Vec<(usize, f64)> {
    let mut neighbors = Vec::new();
    if let Some(rv) = lap.outer_view(row) {
        for (col, &val) in rv.iter() {
            if col != row {
                neighbors.push((col, val.abs()));
            }
        }
    }
    neighbors
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
