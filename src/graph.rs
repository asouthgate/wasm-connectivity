use crate::grid::Grid;

// Compute the harmonic mean of two conductances
// which is used to determine the effective conductance between two nodes in a grid.
fn conductance_avg(a: f64, b: f64) -> f64 {
    let denom = a + b;
    if denom > 0.0 {
        2.0 * a * b / denom
    } else {
        0.0
    }
}

pub struct EdgeTriplets {
    pub row_indices: Vec<usize>,
    pub col_indices: Vec<usize>,
    pub values: Vec<f64>,
}

impl EdgeTriplets {
    pub fn new() -> Self {
        EdgeTriplets {
            row_indices: Vec::new(),
            col_indices: Vec::new(),
            values: Vec::new(),
        }
    }

    // Convenience method to create an EdgeTriplets with pre-allocated capacity
    pub fn with_capacity(capacity: usize) -> Self {
        EdgeTriplets {
            row_indices: Vec::with_capacity(capacity),
            col_indices: Vec::with_capacity(capacity),
            values: Vec::with_capacity(capacity),
        }
    }

    pub fn push(&mut self, i: usize, j: usize, v: f64) {
        self.row_indices.push(i);
        self.col_indices.push(j);
        self.values.push(v);
    }

    pub fn len(&self) -> usize {
        self.row_indices.len()
    }
}

impl Default for EdgeTriplets {
    fn default() -> Self {
        Self::new()
    }
}

// Builds the edge triplets for a grid based on the conductance values and the cell_to_node.
//
// # Arguments
// * `conductance` - A reference to the Grid containing conductance values.
// * `cell_to_node` - A slice containing the mapping from grid indices to node indices (1-based).
// # Returns
// An EdgeTriplets struct containing the edges and their corresponding conductance values.
pub fn build_conductance_edges(conductance: &Grid, cell_to_node: &[i32]) -> EdgeTriplets {
    let nrows = conductance.nrows;
    let ncols = conductance.ncols;

    let estimated_capacity = nrows.saturating_mul(ncols).saturating_mul(4);
    let mut edges = EdgeTriplets::with_capacity(estimated_capacity);

    for row in 0..nrows {
        let row_offset = row * ncols;
        
        let next_row_offset = if row + 1 < nrows {
            (row + 1) * ncols
        } else {
            0 // Will never be read due to the `row + 1 < nrows` guard below
        };

        for col in 0..ncols {
            let node_i = cell_to_node[row_offset + col];
            if node_i <= 0 { // Catch 0 and negative values
                continue;
            }
            let i = (node_i - 1) as usize;
            let g_i = conductance.get(row, col);

            // Check Right Neighbor
            if col + 1 < ncols {
                let node_j = cell_to_node[row_offset + col + 1];
                if node_j > 0 {
                    let j = (node_j - 1) as usize;
                    let g_j = conductance.get(row, col + 1);
                    let g = conductance_avg(g_i, g_j);
                    edges.push(i, j, g);
                    edges.push(j, i, g);
                }
            }

            // Check Down Neighbor
            if row + 1 < nrows {
                let node_j = cell_to_node[next_row_offset + col];
                if node_j > 0 {
                    let j = (node_j - 1) as usize;
                    let g_j = conductance.get(row + 1, col);
                    let g = conductance_avg(g_i, g_j);
                    edges.push(i, j, g);
                    edges.push(j, i, g);
                }
            }
        }
    }

    edges
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_conductance_edges_uniform() {
        let (_cell_to_node, _num_nodes, edges, _lap, _comps) = crate::build_circuit_model(&[1.0; 4], 2, 2, crate::NODATA_SENTINEL);
        assert!(edges.len() > 0);
    }
}
