/// Compute the per-cell current map directly from the resistance raster and
/// the (per-cell) voltage map.
///
/// The current between two adjacent cells `i` and `j` is
///
/// `I_ij = 2 (V_i - V_j) / (r_i + r_j)`,
///
/// The effective resistance is the arithmetic
/// mean `(r_i + r_j) / 2`) (would be harmonic for conductances). 
/// For each cell the map reports the flow magnitude `max(out-flow, in-flow)`.
///
/// Cells whose resistance is nodata, non-finite, or non-positive are treated
/// as non-conductive: they contribute zero current and edges to them are
/// skipped.
///
/// When `neumann_ground` is set, a ground cell contributes its shunt current
/// `ground[i] * V_i` (current to the 0V reference) in addition to any branch
/// currents.
///
/// # Arguments
/// * `resistance_data` - Per-cell resistance raster (row-major).
/// * `ground_data` - Per-cell ground raster (shunt conductance in Neumann mode).
/// * `voltages` - Per-cell voltage map (row-major), length `nrows * ncols`.
/// * `nrows`, `ncols` - Grid dimensions.
/// * `nodata` - The nodata sentinel for `resistance_data`.
/// * `neumann_ground` - Whether `ground_data` declares shunt conductances.
///
/// # Returns
/// Per-cell current map, length `nrows * ncols`.
pub fn compute_current_map_from_raster(
    resistance_data: &[f64],
    ground_data: &[f64],
    voltages: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    neumann_ground: bool,
) -> Vec<f64> {
    let n = nrows * ncols;
    let mut out = vec![0.0f64; n];

    let is_conductive = |r: f64| r.is_finite() && r > 0.0 && r != nodata;

    for ind in 0..n {
        let res = resistance_data[ind];
        if !is_conductive(res) {
            continue;
        }
        let v = voltages[ind];
        let mut pos = 0.0f64;
        let mut neg = 0.0f64;

        // upper neighbor
        if ind >= ncols {
            let nb = ind - ncols;
            if is_conductive(resistance_data[nb]) {
                let branch = 2.0 * (v - voltages[nb]) / (res + resistance_data[nb]);
                if branch > 0.0 { pos += branch; } else { neg -= branch; }
            }
        }
        // lower neighbor
        if ind < (nrows - 1) * ncols {
            let nb = ind + ncols;
            if is_conductive(resistance_data[nb]) {
                let branch = 2.0 * (v - voltages[nb]) / (res + resistance_data[nb]);
                if branch > 0.0 { pos += branch; } else { neg -= branch; }
            }
        }
        // left neighbor
        if ind % ncols != 0 {
            let nb = ind - 1;
            if is_conductive(resistance_data[nb]) {
                let branch = 2.0 * (v - voltages[nb]) / (res + resistance_data[nb]);
                if branch > 0.0 { pos += branch; } else { neg -= branch; }
            }
        }
        // right neighbor
        if (ind + 1) % ncols != 0 {
            let nb = ind + 1;
            if is_conductive(resistance_data[nb]) {
                let branch = 2.0 * (v - voltages[nb]) / (res + resistance_data[nb]);
                if branch > 0.0 { pos += branch; } else { neg -= branch; }
            }
        }

        if neumann_ground && ground_data[ind] > 0.0 {
            let shunt = ground_data[ind] * v;
            if shunt > 0.0 { pos += shunt; } else { neg -= shunt; }
        }

        out[ind] = pos.max(neg);
    }

    out
}

// Retrieve dense 2D grid map from sparse node values and cell_to_node
//
// # Arguments
// * `node_values` - A slice containing the values for each node.
// * `cell_to_node` - A slice containing the mapping from grid indices to node indices (1-based).
// * `n` - The total number of grid points (size of the output vector).
// # Returns
// A vector containing the reconstructed grid values
pub fn reconstruct_grid_map(
    node_values: &[f64],
    cell_to_node: &[i32],
    n: usize,
) -> Vec<f64> {
    debug_assert!(n <= cell_to_node.len(), "Requested grid size `n` exceeds cell_to_node length");
    let mut grid = vec![0.0f64; n];
    for aj in 0..n {
        let node = cell_to_node[aj];
        debug_assert!(node >= 0, "Invalid cell_to_node data: node ID cannot be negative (found {})", node);
        if node != 0 {  // node indices are 1-based in cell_to_node
            grid[aj] = node_values[(node - 1) as usize];
        }
    }
    grid
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconstruct_grid_map() {
        let cell_to_node = vec![0, 1, 2, 0, 0, 3];
        let node_values = vec![10.0, 20.0, 30.0];
        let grid = reconstruct_grid_map(&node_values, &cell_to_node, 2 * 3);
        assert_eq!(grid, vec![0.0, 10.0, 20.0, 0.0, 0.0, 30.0]);
    }

    #[test]
    fn test_reconstruct_grid_map_basic_sparse() {
        let cell_to_node = vec![0, 1, 2, 0, 0, 3];
        let node_values = vec![10.0, 20.0, 30.0];
        let grid = reconstruct_grid_map(&node_values, &cell_to_node, 6);
        assert_eq!(grid, vec![0.0, 10.0, 20.0, 0.0, 0.0, 30.0]);
    }

    #[test]
    fn test_reconstruct_grid_map_shuffled_order() {
        // Nodes don't have to be sequential
        let cell_to_node = vec![3, 0, 1, 0, 2];
        let node_values = vec![100.0, 200.0, 300.0]; // Node 1=100, Node 2=200, Node 3=300
        let grid = reconstruct_grid_map(&node_values, &cell_to_node, 5);
        assert_eq!(grid, vec![300.0, 0.0, 100.0, 0.0, 200.0]);
    }

    #[test]
    fn test_compute_current_map_uniform() {
        // 2x2 uniform grid, alternating voltages: each cell carries 1.0 A.
        let resistance = vec![1.0, 1.0, 1.0, 1.0];
        let ground = vec![0.0, 0.0, 0.0, 0.0];
        let voltages = vec![1.0, 0.0, 1.0, 0.0];
        let out = compute_current_map_from_raster(
            &resistance, &ground, &voltages, 2, 2, crate::NODATA_SENTINEL, false,
        );
        assert_eq!(out, vec![1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn test_compute_current_map_nodata() {
        // Cell 1 is nodata: it reports 0, and its neighbours skip the edge.
        let resistance = vec![1.0, crate::NODATA_SENTINEL, 1.0, 1.0];
        let ground = vec![0.0, 0.0, 0.0, 0.0];
        let voltages = vec![1.0, 0.0, 1.0, 0.0];
        let out = compute_current_map_from_raster(
            &resistance, &ground, &voltages, 2, 2, crate::NODATA_SENTINEL, false,
        );
        assert_eq!(out, vec![0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn test_compute_current_map_neumann_ground() {
        // 2x1 grid, ground (shunt conductance 5) on the bottom cell at V=1.
        let resistance = vec![1.0, 1.0];
        let ground = vec![0.0, 5.0];
        let voltages = vec![2.0, 1.0];

        let out_no_shunt = compute_current_map_from_raster(
            &resistance, &ground, &voltages, 2, 1, crate::NODATA_SENTINEL, false,
        );
        assert_eq!(out_no_shunt, vec![1.0, 1.0]);

        let out_shunt = compute_current_map_from_raster(
            &resistance, &ground, &voltages, 2, 1, crate::NODATA_SENTINEL, true,
        );
        // Bottom cell adds shunt 5 * 1 = 5 A on top of the 1 A branch current.
        assert_eq!(out_shunt, vec![1.0, 5.0]);
    }
}
