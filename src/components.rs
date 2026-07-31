use sprs::{CsMat, TriMat};
use std::collections::HashMap;
use crate::laplacian::regularize_laplacian;


/// Finds the connected components in a graph represented by its Laplacian matrix.
///
/// This function performs a depth-first search to identify all connected components
/// in the graph.
///
/// # Arguments
///
/// * `laplacian` - A reference to a `CsMat<f64>` representing the Laplacian matrix.
/// * `num_nodes` - The total number of nodes in the graph.
/// # Returns
/// A vector of tuples of indexes, each representing a component of the graph.
pub fn find_connected_components(
    laplacian: &CsMat<f64>,
    num_nodes: usize,
) -> Vec<Vec<usize>> {
    let mut visited = vec![false; num_nodes];
    let mut components: Vec<Vec<usize>> = Vec::new();
    let mut stack = Vec::with_capacity(64);

    for start in 0..num_nodes {
        if visited[start] {
            continue;
        }
        let mut comp = Vec::new();
        
        stack.clear();
        stack.push(start);
        visited[start] = true;

        while let Some(node) = stack.pop() {
            comp.push(node);
            if let Some(rv) = laplacian.outer_view(node) {
                for (neighbor, _) in rv.iter() {
                    if neighbor != node && !visited[neighbor] {
                        visited[neighbor] = true;
                        stack.push(neighbor);
                    }
                }
            }
        }
        components.push(comp);
    }

    components
}

/// Builds the Laplacian matrix for a subgraph defined by a set of nodes.
///
/// # Arguments
///
/// * `parent_laplacian` - A reference to the parent Laplacian matrix.
/// * `comp` - A slice of node indices representing the subgraph component.
/// * `num_nodes` - The total number of nodes in the parent graph.
/// # Returns
/// A tuple containing the Laplacian matrix of the subgraph
pub fn build_subgraph_laplacian(
    parent_laplacian: &CsMat<f64>,
    comp: &[usize],
) -> (CsMat<f64>, HashMap<usize, usize>) {
    let comp_size = comp.len();
    let mut node_to_local: HashMap<usize, usize> = HashMap::with_capacity(comp_size);
    for (local_idx, &global_node) in comp.iter().enumerate() {
        node_to_local.insert(global_node, local_idx);
    }

    let mut local_tri = TriMat::new((comp_size, comp_size));
    let mut local_diags = vec![0.0f64; comp_size];
    let mut local_row_sums = vec![0.0f64; comp_size];

    // The parent's diagonal may include this crate's regularization bump at
    // global node 0 (see regularize_laplacian). Recover the exact bump so the
    // preserved diagonal can exclude it; the subgraph is re-regularized
    // below, leaving precisely one bump. The bump was computed on the
    // *unbumped* matrix, so one fixed-point step is needed to recover that
    // norm from the bumped matrix we see here.
    let (s_bumped, pd00) = {
        let mut s = 0.0f64;
        let mut pd = 0.0f64;
        for i in 0..parent_laplacian.rows() {
            if let Some(rv) = parent_laplacian.outer_view(i) {
                for (col, &v) in rv.iter() {
                    s += v * v;
                    if i == 0 && col == 0 {
                        pd = v;
                    }
                }
            }
        }
        (s, pd)
    };
    let parent_bump = if s_bumped.is_finite() && s_bumped > 0.0 && pd00 != 0.0 {
        let cpb = 1e-5 * s_bumped.sqrt();
        let s_pre = s_bumped - pd00 * pd00 + (pd00 - cpb) * (pd00 - cpb);
        1e-5 * s_pre.max(0.0).sqrt()
    } else {
        0.0
    };

    for &global_u in comp {
        let local_u = match node_to_local.get(&global_u) {
            Some(&v) => v,
            None => continue,
        };
        let row_view = match parent_laplacian.outer_view(global_u) {
            Some(rv) => rv,
            None => continue,
        };
        let mut parent_diag: Option<f64> = None;
        for (global_v, &parent_val) in row_view.indices().iter().zip(row_view.data()) {
            if global_u == *global_v {
                parent_diag = Some(parent_val);
                continue;
            }
            let local_v = match node_to_local.get(global_v) {
                Some(&v) => v,
                None => continue,
            };
            // off-diagonal element, preserve its exact negative value from the parent matrix.
            local_tri.add_triplet(local_u, local_v, parent_val);
            local_row_sums[local_u] += parent_val.abs();
        }
        // Preserve the parent's diagonal entry as-is. Besides the edge
        // conductance sum it may carry terms that are not representable by
        // within-component edges e.g. finite ground (shunt) conductances
        // declared on the parent diagonal, which must survive extraction.
        local_diags[local_u] = match parent_diag {
            Some(pd) if global_u == 0 && parent_bump != 0.0 => pd - parent_bump,
            Some(pd) => pd,
            None => local_row_sums[local_u],
        };
    }

    // Add diagonal elements
    for (local_u, &diag) in local_diags.iter().enumerate() {
        local_tri.add_triplet(local_u, local_u, diag);
    }

    // Convert/compress to csr format
    let mut local_lap = local_tri.to_csr();
    regularize_laplacian(&mut local_lap);
    (local_lap, node_to_local)
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_connected_components_single() {
        let (_cell_to_node, _num_nodes, _edges, _lap, comps) = crate::build_circuit_model(&[1.0; 9], 3, 3, crate::NODATA_SENTINEL);
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0].len(), 9);
    }

    #[test]
    fn test_connected_components_disconnected() {
        let res_data = vec![
            1.0, 0.0, 1.0,
            0.0, 0.0, 0.0,
            1.0, 0.0, 1.0,
        ];
        let (_cell_to_node, _num_nodes, _edges, _lap, comps) = crate::build_circuit_model(&res_data, 3, 3, 0.0);
        assert_eq!(comps.len(), 4);
    }
}
