use crate::real::{Real, scalar};
use sprs::CsMat;
use std::collections::HashSet;
use super::sources::valid_positive_value;

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum GroundMode {
    /// Conductance-to-ground (Shunt): ground cells add conductance to Laplacian diagonal.
    Shunt,
    /// Fixed-voltage (Dirichlet): ground cells are pinned at V=0.
    Dirichlet,
}

/// Collect global node indices where ground_data > 0.
pub(super) fn collect_ground_nodes<F: Real>(
    cell_to_node: &[i32],
    ground_data: &[F],
    nrows: usize,
    ncols: usize,
    nodata: F,
) -> Vec<usize> {
    let mut nodes = Vec::new();
    for row in 0..nrows {
        for col in 0..ncols {
            let idx = row * ncols + col;
            let node = cell_to_node[idx];
            if node > 0 {
                let gv = ground_data[idx];
                if valid_positive_value(gv, nodata) {
                    nodes.push((node - 1) as usize);
                }
            }
        }
    }
    nodes.sort();
    nodes.dedup();
    nodes
}

/// Apply Dirichlet BC (V=0) at ground nodes.
/// Returns a new Laplacian with ground-node rows replaced by identity
/// and ground-node columns removed from non-ground rows.
/// The caller must also zero the source vector at ground entries.
pub(crate) fn apply_dirichlet_ground_lap<F: Real>(
    laplacian: &CsMat<F>,
    ground_nodes: &[usize],
) -> CsMat::<F> {
    let n = laplacian.rows();
    let ground_set: HashSet<usize> = ground_nodes.iter().copied().collect();

    let mut rows = Vec::new();
    let mut cols = Vec::new();
    let mut vals = Vec::new();

    for row in 0..n {
        if let Some(rv) = laplacian.outer_view(row) {
            if ground_set.contains(&row) {
                rows.push(row);
                cols.push(row);
                vals.push(scalar::<F>(1.0));
            } else {
                for (col, &val) in rv.iter() {
                    if !ground_set.contains(&col) {
                        rows.push(row);
                        cols.push(col);
                        vals.push(val);
                    }
                }
            }
        }
    }

    sprs::TriMat::from_triplets((n, n), rows, cols, vals).to_csr()
}

/// Zero out source entries at ground node positions.
pub(crate) fn zero_ground_rhs<F: Real>(s: &mut [F], ground_nodes: &[usize]) {
    for &gn in ground_nodes {
        if gn < s.len() {
            s[gn] = scalar::<F>(0.0);
        }
    }
}

/// Per-node Dirichlet pinned mask (node index == flat cell index).
pub(super) fn build_dirichlet_mask_identity<F: Real>(ground_data: &[F], nodata: F) -> Vec<bool> {
    ground_data
        .iter()
        .map(|&gv| valid_positive_value(gv, nodata))
        .collect()
}
