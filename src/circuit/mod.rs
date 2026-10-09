pub mod grid;
pub mod graph;
pub mod laplacian;

/// Build the sparse circuit model (conductance grid -> graph edges -> Laplacian)
/// from a resistance raster.
///
/// Returns `(cell_to_node, num_nodes, edges, laplacian)` where `cell_to_node`
/// maps raster index -> 1-based node id (0 for non-conductive cells).
pub fn build_circuit_model(
    resistance_data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
) -> (Vec<i32>, usize, graph::EdgeTriplets, sprs::CsMat<f64>) {
    let conductance = grid::Grid::to_conductance(resistance_data, nrows, ncols, nodata);
    crate::memory::record_conductance_grid(nrows * ncols);
    let (cell_to_node, num_nodes) = grid::build_cell_to_node(&conductance);
    crate::memory::record_cell_to_node_map(nrows * ncols);
    let edges = graph::build_conductance_edges(&conductance, &cell_to_node);
    crate::memory::record_edge_triplets(edges.len());
    let laplacian = laplacian::build_laplacian(&edges, num_nodes);
    (cell_to_node, num_nodes, edges, laplacian)
}
