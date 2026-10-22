use crate::linalg::real::{Real};
pub mod grid;
pub mod graph;
pub mod laplacian;

/// Build only the matrix and node mapping, without materialising graph edges.
pub(crate) fn build_circuit_matrix<F: Real>(
    resistance_data: &[F],
    nrows: usize,
    ncols: usize,
    nodata: F,
) -> (Vec<i32>, usize, sprs::CsMat<F>) {
    let conductance = grid::Grid::to_conductance(resistance_data, nrows, ncols, nodata);
    crate::memory::record_conductance_grid_typed::<F>(nrows * ncols);
    let (cell_to_node, num_nodes) = grid::build_cell_to_node(&conductance);
    crate::memory::record_cell_to_node_map(nrows * ncols);
    let laplacian = laplacian::build_grid_laplacian(&conductance, &cell_to_node, num_nodes);
    crate::memory::record_assembly_matrix(&laplacian);
    (cell_to_node, num_nodes, laplacian)
}

/// Build the sparse circuit model (conductance grid -> graph edges -> Laplacian)
/// from a resistance raster.
///
/// Returns `(cell_to_node, num_nodes, edges, laplacian)` where `cell_to_node`
/// maps raster index -> 1-based node id (0 for non-conductive cells).
pub fn build_circuit_model<F: Real>(
    resistance_data: &[F],
    nrows: usize,
    ncols: usize,
    nodata: F,
) -> (Vec<i32>, usize, graph::EdgeTriplets<F>, sprs::CsMat<F>) {
    let conductance = grid::Grid::to_conductance(resistance_data, nrows, ncols, nodata);
    crate::memory::record_conductance_grid_typed::<F>(nrows * ncols);
    let (cell_to_node, num_nodes) = grid::build_cell_to_node(&conductance);
    crate::memory::record_cell_to_node_map(nrows * ncols);
    let edges = graph::build_conductance_edges(&conductance, &cell_to_node);
    crate::memory::record_edge_triplets_typed::<F>(edges.len());
    let laplacian = laplacian::build_laplacian(&edges, num_nodes);
    (cell_to_node, num_nodes, edges, laplacian)
}
