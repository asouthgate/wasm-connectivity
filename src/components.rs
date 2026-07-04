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
        let mut queue = vec![start];
        visited[start] = true;

        while let Some(node) = queue.pop() {
            comp.push(node);
            for (neighbor, _) in get_row_neighbors(laplacian, node) {
                if !visited[neighbor] {
                    visited[neighbor] = true;
                    queue.push(neighbor);
                }
            }
        }

        if !comp.is_empty() {
            components.push(comp);
        }
    }

    components
}
