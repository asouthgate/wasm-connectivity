use crate::grid::Grid;

fn conductance_avg(a: f64, b: f64) -> f64 {
    (a + b) / 2.0
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

    pub fn push(&mut self, i: usize, j: usize, v: f64) {
        self.row_indices.push(i);
        self.col_indices.push(j);
        self.values.push(v);
    }

    pub fn len(&self) -> usize {
        self.row_indices.len()
    }
}

pub fn build_adjacency(conductance: &Grid, nodemap: &[i32]) -> EdgeTriplets {
    let mut edges = EdgeTriplets::new();
    let nrows = conductance.nrows;
    let ncols = conductance.ncols;

    for row in 0..nrows {
        for col in 0..ncols {
            let node_i = nodemap[row * ncols + col];
            if node_i == 0 {
                continue;
            }
            let i = (node_i - 1) as usize;
            let g_i = conductance.get(row, col);

            if col + 1 < ncols {
                let node_j = nodemap[row * ncols + col + 1];
                if node_j != 0 {
                    let j = (node_j - 1) as usize;
                    let g_j = conductance.get(row, col + 1);
                    let g = conductance_avg(g_i, g_j);
                    edges.push(i, j, g);
                    edges.push(j, i, g);
                }
            }

            if row + 1 < nrows {
                let node_j = nodemap[(row + 1) * ncols + col];
                if node_j != 0 {
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
