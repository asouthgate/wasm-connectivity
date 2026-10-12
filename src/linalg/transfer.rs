//! Bilinear geometric transfers: dimensions are stored, entries are generated.

// One-dimensional interpolation coefficients at fine offsets -1, 0, 1, 2
// from twice a coarse coordinate. Two-dimensional weights are their products.
const STENCIL_WEIGHTS: [f64; 4] = [0.25, 0.75, 0.75, 0.25];
const ROW_WEIGHTS: [[f64; 2]; 2] = [
    [STENCIL_WEIGHTS[3], STENCIL_WEIGHTS[1]],
    [STENCIL_WEIGHTS[2], STENCIL_WEIGHTS[0]],
];

#[derive(Clone, Copy)]
pub(super) struct Transfer {
    pub fine_nrows: usize,
    pub fine_ncols: usize,
    pub coarse_nrows: usize,
    pub coarse_ncols: usize,
}

impl Transfer {
    pub fn coarse_n(&self) -> usize {
        self.coarse_nrows * self.coarse_ncols
    }

    /// P maps coarse-grid values to fine-grid values, so a column selects one
    /// coarse cell and visits the fine cells it influences, in fine-index order.
    /// In P^T A P, this supplies P[a, i] for a selected coarse index i.
    /// Out-of-grid contributions are omitted; retained coefficients are unchanged.
    #[inline]
    pub fn for_each_column(&self, coarse: usize, mut f: impl FnMut(usize, f64)) {
        let cr = coarse / self.coarse_ncols;
        let cc = coarse % self.coarse_ncols;
        for (ri, &rw) in STENCIL_WEIGHTS.iter().enumerate() {
            let fr = 2 * cr as isize + ri as isize - 1;
            if fr < 0 || fr >= self.fine_nrows as isize {
                continue;
            }
            for (ci, &cw) in STENCIL_WEIGHTS.iter().enumerate() {
                let fc = 2 * cc as isize + ci as isize - 1;
                if fc < 0 || fc >= self.fine_ncols as isize {
                    continue;
                }
                f(fr as usize * self.fine_ncols + fc as usize, rw * cw);
            }
        }
    }

    /// A row selects one fine cell and visits the coarse cells contributing to
    /// it, in coarse-index order. In P^T A P, this supplies P[b, j] for a fine
    /// index b reached through A[a, b]; columns instead select coarse indices.
    #[inline]
    pub fn for_each_row(&self, fine: usize, mut f: impl FnMut(usize, f64)) {
        self.for_each_row_coordinate(fine, |cr, cc, weight| {
            f(cr * self.coarse_ncols + cc, weight);
        });
    }

    /// Coordinate form avoids dividing generated coarse indices again when
    /// accumulating into a local stencil centred on a known coarse cell.
    #[inline]
    pub fn for_each_row_coordinate(&self, fine: usize, mut f: impl FnMut(usize, usize, f64)) {
        let fr = fine / self.fine_ncols; // Fine row.
        let fc = fine % self.fine_ncols; // Fine column.

        // For an even fine coordinate 2k, coarse neighbours (k-1, k)
        // contribute via stencil offsets (2, 0), at coefficient indices (3, 1).
        // For an odd coordinate 2k+1, neighbours (k, k+1) use offsets (1, -1),
        // at coefficient indices (2, 0). Parity selects
        // the corresponding coefficients, ordered by coarse coordinate.
        let rstart = ((fr + 1) / 2) as isize - 1;
        let cstart = ((fc + 1) / 2) as isize - 1;
        let rw = ROW_WEIGHTS[fr % 2];
        let cw = ROW_WEIGHTS[fc % 2];
        for (ri, &rweight) in rw.iter().enumerate() {
            let cr = rstart + ri as isize;
            if cr < 0 || cr >= self.coarse_nrows as isize {
                continue;
            }
            for (ci, &cweight) in cw.iter().enumerate() {
                let cc = cstart + ci as isize;
                if cc < 0 || cc >= self.coarse_ncols as isize {
                    continue;
                }
                f(cr as usize, cc as usize, rweight * cweight);
            }
        }
    }

    /// fine += P * coarse.
    pub fn prolongate(&self, coarse: &[f64], fine: &mut [f64]) {
        for (col, &value) in coarse.iter().enumerate() {
            self.for_each_column(col, |row, weight| fine[row] += weight * value);
        }
    }

    /// coarse = P^T * fine.
    pub fn restrict(&self, fine: &[f64], coarse: &mut [f64]) {
        for (col, value) in coarse.iter_mut().enumerate() {
            *value = 0.0;
            self.for_each_column(col, |row, weight| *value += weight * fine[row]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference_matrix(transfer: &Transfer) -> sprs::CsMat<f64> {
        let nr = transfer.fine_nrows;
        let nc = transfer.fine_ncols;
        let mut reference = sprs::TriMat::new((nr * nc, transfer.coarse_n()));
        let weights = [0.25, 0.75, 0.75, 0.25];
        for cr in 0..nr / 2 {
            for cc in 0..nc / 2 {
                for dr in 0..4 {
                    for dc in 0..4 {
                        let r = 2 * cr as isize + dr as isize - 1;
                        let c = 2 * cc as isize + dc as isize - 1;
                        if r >= 0 && c >= 0 && r < nr as isize && c < nc as isize {
                            reference.add_triplet(
                                r as usize * nc + c as usize,
                                cr * (nc / 2) + cc,
                                weights[dr] * weights[dc],
                            );
                        }
                    }
                }
            }
        }
        reference.to_csr::<usize>()
    }

    fn assert_entries_match(transfer: &Transfer, csr: &sprs::CsMat<f64>) {
        let csc = csr.to_csc();
        for row in 0..transfer.fine_nrows * transfer.fine_ncols {
            let mut actual = Vec::new();
            transfer.for_each_row(row, |col, val| actual.push((col, val)));
            assert_eq!(
                actual,
                csr.outer_view(row)
                    .unwrap()
                    .iter()
                    .map(|(col, &val)| (col, val))
                    .collect::<Vec<_>>()
            );
        }
        for col in 0..transfer.coarse_n() {
            let mut actual = Vec::new();
            transfer.for_each_column(col, |row, val| actual.push((row, val)));
            assert_eq!(
                actual,
                csc.outer_view(col)
                    .unwrap()
                    .iter()
                    .map(|(row, &val)| (row, val))
                    .collect::<Vec<_>>()
            );
        }
    }

    fn assert_transfers_and_transpose(transfer: &Transfer, csr: &sprs::CsMat<f64>) {
        let nr = transfer.fine_nrows;
        let nc = transfer.fine_ncols;
        let fine: Vec<f64> = (0..nr * nc).map(|i| (i as f64 * 0.7).sin()).collect();
        let coarse: Vec<f64> = (0..transfer.coarse_n())
            .map(|i| (i as f64 * 0.3).cos())
            .collect();
        let mut restricted = vec![0.0; coarse.len()];
        let mut prolonged = vec![0.0; fine.len()];
        transfer.restrict(&fine, &mut restricted);
        transfer.prolongate(&coarse, &mut prolonged);
        let left: f64 = restricted.iter().zip(&coarse).map(|(a, b)| a * b).sum();
        let right: f64 = fine.iter().zip(&prolonged).map(|(a, b)| a * b).sum();
        assert!((left - right).abs() < 1e-12);
        let mut expected_fine = vec![0.0; fine.len()];
        let mut expected_coarse = vec![0.0; coarse.len()];
        for (row, entries) in csr.outer_iterator().enumerate() {
            for (col, &val) in entries.iter() {
                expected_fine[row] += val * coarse[col];
                expected_coarse[col] += val * fine[row];
            }
        }
        for (a, b) in prolonged.iter().zip(expected_fine) {
            assert!((a - b).abs() < 1e-12);
        }
        for (a, b) in restricted.iter().zip(expected_coarse) {
            assert!((a - b).abs() < 1e-12);
        }
        assert_additive_prolongation(transfer, &coarse, &prolonged);
    }

    fn assert_additive_prolongation(transfer: &Transfer, coarse: &[f64], prolonged: &[f64]) {
        let mut offset = vec![2.0; prolonged.len()];
        transfer.prolongate(coarse, &mut offset);
        for (a, b) in offset.iter().zip(prolonged) {
            assert!((a - 2.0 - b).abs() < 1e-12);
        }
    }

    #[test]
    fn transfers_match_triplets_and_are_transposes() {
        for (nr, nc) in [(2, 2), (3, 5), (8, 8), (15, 16), (16, 15), (2, 19), (19, 2)] {
            let transfer = Transfer {
                fine_nrows: nr,
                fine_ncols: nc,
                coarse_nrows: nr / 2,
                coarse_ncols: nc / 2,
            };
            let reference = reference_matrix(&transfer);
            assert_entries_match(&transfer, &reference);
            assert_transfers_and_transpose(&transfer, &reference);
        }
    }
}
