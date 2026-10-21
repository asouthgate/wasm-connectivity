use crate::linalg::real::{Real, scalar};
use sprs::CsMat;
use crate::circuit::graph::EdgeTriplets;

pub(crate) fn build_grid_laplacian<F: Real>(
    conductance: &crate::circuit::grid::Grid<F>,
    cell_to_node: &[i32],
    num_nodes: usize,
) -> CsMat::<F> {
    let nr = conductance.nrows;
    let nc = conductance.ncols;
    let neighbors = |row: usize, col: usize| {
        [
            row.checked_sub(1).map(|r| r * nc + col),
            col.checked_sub(1).map(|c| row * nc + c),
            (col + 1 < nc).then_some(row * nc + col + 1),
            (row + 1 < nr).then_some((row + 1) * nc + col),
        ]
    };

    // Count structural entries, then allocate the final CSR arrays exactly once.
    let mut indptr = Vec::with_capacity(num_nodes + 1);
    indptr.push(0);
    for row in 0..nr {
        for col in 0..nc {
            if cell_to_node[row * nc + col] <= 0 {
                continue;
            }
            let degree = neighbors(row, col).into_iter().flatten()
                .filter(|&idx| cell_to_node[idx] > 0).count();
            indptr.push(indptr.last().unwrap() + degree + 1);
        }
    }
    let nnz = *indptr.last().unwrap();
    let mut indices = Vec::with_capacity(nnz);
    let mut values = Vec::with_capacity(nnz);
    for row in 0..nr {
        for col in 0..nc {
            let idx = row * nc + col;
            let node = cell_to_node[idx];
            if node <= 0 {
                continue;
            }
            let mut entries = [(0, scalar::<F>(0.0)); 4];
            let mut count = 0;
            let mut diagonal = scalar::<F>(0.0);
            // Up, left, right, down matches the old edge accumulation order.
            for neighbor in neighbors(row, col).into_iter().flatten() {
                let other = cell_to_node[neighbor];
                if other <= 0 {
                    continue;
                }
                let (first, second) = if neighbor < idx { (neighbor, idx) } else { (idx, neighbor) };
                let g = crate::circuit::graph::conductance_avg(
                    conductance.data[first], conductance.data[second],
                );
                diagonal += g;
                entries[count] = ((other - 1) as usize, -g);
                count += 1;
            }
            let node = (node - 1) as usize;
            let mut inserted_diagonal = false;
            for &(other, value) in &entries[..count] {
                if !inserted_diagonal && other > node {
                    indices.push(node);
                    values.push(diagonal);
                    inserted_diagonal = true;
                }
                indices.push(other);
                values.push(value);
            }
            if !inserted_diagonal {
                indices.push(node);
                values.push(diagonal);
            }
        }
    }
    CsMat::new((num_nodes, num_nodes), indptr, indices, values)
}

pub fn extract_diag_inv<F: Real>(lap: &CsMat<F>) -> Vec<F> {
    let n = lap.rows();
    let mut diag_inv = vec![scalar::<F>(0.0); n];
    for row in 0..n {
        if let Some(rv) = lap.outer_view(row) {
            for (col, &val) in rv.iter() {
                if col == row {
                    let abs_val = val.abs();
                    diag_inv[row] = if abs_val > scalar::<F>(1e-15) { scalar::<F>(1.0) / abs_val } else { scalar::<F>(0.0) };
                    break;
                }
            }
        }
    }
    diag_inv
}

// Regularizes the Laplacian matrix by adding a small value to the diagonal elements,
// in order to ensure numerical stability and avoid singular matrices.
pub fn regularize_laplacian<F: Real>(lap: &mut CsMat<F>) {
    let norm: F = lap.data().iter().map(|&v| v * v).sum::<F>().sqrt();
    if !norm.is_finite() || norm <= scalar::<F>(0.0) {
        return;
    }
    
    // Explicitly ground the first node on every level.
    // This creates a reliable, invariant potential reference across the entire hierarchy.
    if let Some(mut row_vec) = lap.outer_view_mut(0) {
        for (col, val) in row_vec.iter_mut() {
            if col == 0 {
                *val += scalar::<F>(1e-5) * norm;
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
pub fn build_laplacian<F: Real>(edges: &EdgeTriplets<F>, num_nodes: usize) -> CsMat::<F> {
    let mut row_sums = vec![scalar::<F>(0.0); num_nodes];

    let nnz = edges.len() + num_nodes;
    crate::memory::record_assembly_scratch_typed::<F>(num_nodes, nnz);
    let mut lap_rows: Vec<usize> = Vec::with_capacity(nnz);
    let mut lap_cols: Vec<usize> = Vec::with_capacity(nnz);
    let mut lap_vals: Vec<F> = Vec::with_capacity(nnz);

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
    tri.to_csr()
}

/// Add a per-node diagonal to a matrix: returns `lap` with `A[i,i] += diag[i]`.
///
/// Used to declare the full system matrix up front, e.g. `L + G` where `G`
/// holds finite ground (shunt) conductances on the diagonal.
pub fn add_diagonal<F: Real>(lap: &CsMat<F>, diag: &[F]) -> CsMat::<F> {
    let n = lap.rows();
    let mut rows = Vec::new();
    let mut cols = Vec::new();
    let mut vals = Vec::new();
    for row in 0..n {
        let boost = if row < diag.len() { diag[row] } else { scalar::<F>(0.0) };
        if let Some(rv) = lap.outer_view(row) {
            let mut found_diag = false;
            for (col, &val) in rv.iter() {
                if col == row {
                    rows.push(row);
                    cols.push(col);
                    vals.push(val + boost);
                    found_diag = true;
                } else {
                    rows.push(row);
                    cols.push(col);
                    vals.push(val);
                }
            }
            if !found_diag && boost != scalar::<F>(0.0) {
                rows.push(row);
                cols.push(row);
                vals.push(boost);
            }
        } else if boost != scalar::<F>(0.0) {
            rows.push(row);
            cols.push(row);
            vals.push(boost);
        }
    }
    sprs::TriMat::from_triplets((n, n), rows, cols, vals).to_csr()
}

/// Update the stored diagonal of a circuit Laplacian without rebuilding it.
/// `build_laplacian` stores a diagonal entry for every node, including isolated nodes.
pub(crate) fn add_diagonal_in_place<F: Real>(lap: &mut CsMat<F>, diag: &[F]) {
    assert_eq!(lap.rows(), lap.cols());
    assert_eq!(lap.rows(), diag.len());
    for (row, &boost) in diag.iter().enumerate() {
        *lap.get_mut(row, row).expect("circuit Laplacian must store every diagonal") += boost;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::graph::EdgeTriplets;

    #[test]
    fn direct_grid_csr_matches_triplet_assembly() {
        let nodata = -9999.0;
        for (nr, nc) in [(0, 0), (1, 1), (1, 9), (9, 1), (2, 2), (7, 11)] {
            for pattern in 0..3 {
                let resistance: Vec<f64> = (0..nr * nc).map(|idx| {
                    if pattern == 2 {
                        return nodata;
                    }
                    if pattern == 1 {
                        match idx % 7 {
                            0 => return nodata,
                            1 => return 0.0,
                            2 => return -1.0,
                            3 => return f64::NAN,
                            4 => return f64::INFINITY,
                            _ => {}
                        }
                    }
                    10.0f64.powi((idx % 23) as i32 - 11) * (1.0 + idx as f64 / 137.0)
                }).collect();
                let (old_map, old_nodes, _, expected) =
                    crate::build_circuit_model(&resistance, nr, nc, nodata);
                let (new_map, new_nodes, actual) =
                    crate::circuit::build_circuit_matrix(&resistance, nr, nc, nodata);
                assert_eq!(new_map, old_map);
                assert_eq!(new_nodes, old_nodes);
                assert_eq!(actual, expected, "{nr}x{nc}, pattern {pattern}");
                for (&a, &b) in actual.data().iter().zip(expected.data()) {
                    assert_eq!(a.to_bits(), b.to_bits());
                }
            }
        }
    }

    #[test]
    fn grounding_in_place_matches_rebuild_and_reuses_storage() {
        let mut edges = EdgeTriplets::new();
        for (i, j, value) in [(0, 1, 2.5), (1, 0, 2.5), (1, 2, 0.125), (2, 1, 0.125)] {
            edges.push(i, j, value);
        }
        let original = build_laplacian(&edges, 4);
        for diagonal in [[0.0; 4], [1e-20, 3.0, 0.0, 7.0]] {
            let expected = add_diagonal(&original, &diagonal);
            let mut actual = original.clone();
            let data = actual.data().as_ptr();
            let indices = actual.indices().as_ptr();
            let indptr = actual.indptr().raw_storage().as_ptr();
            add_diagonal_in_place(&mut actual, &diagonal);
            assert_eq!(actual, expected);
            assert_eq!(actual.data().as_ptr(), data);
            assert_eq!(actual.indices().as_ptr(), indices);
            assert_eq!(actual.indptr().raw_storage().as_ptr(), indptr);
        }
    }

    #[test]
    fn test_build_laplacian_two_node() {
        let mut edges = EdgeTriplets::new();
        edges.push(0, 1, 5.0);
        edges.push(1, 0, 5.0);
        let lap = build_laplacian(&edges, 2);
        assert_eq!(lap.rows(), 2);
        assert_eq!(lap.cols(), 2);
        for row in lap.outer_iterator() {
            assert!(row.data().iter().sum::<f64>().abs() < 1e-10);
        }

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
