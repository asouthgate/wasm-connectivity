use sprs::CsMat;
use crate::laplacian::get_row_neighbors;

pub fn find_connected_components(
    laplacian: &CsMat<f64>,
    num_nodes: usize,
) -> Vec<Vec<usize>> {
    let mut visited = vec![false; num_nodes];
    let mut components: Vec<Vec<usize>> = Vec::new();

    for start in 0..num_nodes {
        if visited[start] {
            continue;
        }
        let mut comp = Vec::new();
        let mut stack = vec![start];
        visited[start] = true;

        while let Some(node) = stack.pop() {
            comp.push(node);
            for (neighbor, _) in get_row_neighbors(laplacian, node) {
                if !visited[neighbor] {
                    visited[neighbor] = true;
                    stack.push(neighbor);
                }
            }
        }

        if !comp.is_empty() {
            components.push(comp);
        }
    }

    components
}

pub fn build_subgraph_laplacian(
    laplacian: &CsMat<f64>,
    comp: &[usize],
    num_nodes: usize,
) -> (CsMat<f64>, Vec<usize>) {
    let comp_size = comp.len();
    let comp_set: std::collections::HashSet<usize> = comp.iter().copied().collect();

    let mut node_to_local = vec![0usize; num_nodes];
    for (li, &gn) in comp.iter().enumerate() {
        node_to_local[gn] = li;
    }

    let mut a_local_triplets = crate::graph::EdgeTriplets::new();
    for &gn in comp {
        let li = node_to_local[gn];
        for (neighbor, conductance) in get_row_neighbors(laplacian, gn) {
            if comp_set.contains(&neighbor) && neighbor > gn {
                let lj = node_to_local[neighbor];
                a_local_triplets.push(li, lj, conductance);
                a_local_triplets.push(lj, li, conductance);
            }
        }
    }

    let a_local = crate::laplacian::build_laplacian(&a_local_triplets, comp_size);
    (a_local, node_to_local)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph;
    use crate::laplacian;
    use crate::grid;

    #[test]
    fn test_connected_components_single() {
        let cond = grid::Grid::to_conductance(&vec![1.0; 9], 3, 3, -9999.0);
        let (nodemap, num_nodes) = grid::build_nodemap(&cond);
        let edges = graph::build_conductance_edges(&cond, &nodemap);
        let lap = laplacian::build_laplacian(&edges, num_nodes);
        let comps = find_connected_components(&lap, num_nodes);
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
        let cond = grid::Grid::to_conductance(&res_data, 3, 3, 0.0);
        let (nodemap, num_nodes) = grid::build_nodemap(&cond);
        let edges = graph::build_conductance_edges(&cond, &nodemap);
        let lap = laplacian::build_laplacian(&edges, num_nodes);
        let comps = find_connected_components(&lap, num_nodes);
        assert_eq!(comps.len(), 4);
    }
}
