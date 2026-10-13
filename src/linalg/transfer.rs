//! Node-centred bilinear transfers: dimensions are stored, entries are generated.

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
    /// Out-of-grid contributions are omitted; unmatched final fine points copy
    /// the nearest coarse endpoint.
    #[inline]
    pub fn for_each_column(&self, coarse: usize, mut f: impl FnMut(usize, f64)) {
        let cr = coarse / self.coarse_ncols;
        let cc = coarse % self.coarse_ncols;
        for (ri, &rw) in [0.5, 1.0, 0.5].iter().enumerate() {
            let fr = 2 * cr as isize + ri as isize - 1;
            if fr < 0 || fr >= self.fine_nrows as isize {
                continue;
            }
            for (ci, &cw) in [0.5, 1.0, 0.5].iter().enumerate() {
                let fc = 2 * cc as isize + ci as isize - 1;
                if fc < 0 || fc >= self.fine_ncols as isize {
                    continue;
                }
                let rw = if fr as usize == self.fine_nrows - 1 && ri == 2 {
                    1.0
                } else {
                    rw
                };
                let cw = if fc as usize == self.fine_ncols - 1 && ci == 2 {
                    1.0
                } else {
                    cw
                };
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
        let fr = fine / self.fine_ncols;
        let fc = fine % self.fine_ncols;
        let r0 = fr / 2;
        let c0 = fc / 2;
        let r1 = (r0 + fr % 2).min(self.coarse_nrows - 1);
        let c1 = (c0 + fc % 2).min(self.coarse_ncols - 1);
        let weight = 1.0 / ((r1 - r0 + 1) * (c1 - c0 + 1)) as f64;
        for cr in r0..=r1 {
            for cc in c0..=c1 {
                f(cr, cc, weight);
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
        for r in 0..nr {
            for c in 0..nc {
                let cr = r / 2;
                let cc = c / 2;
                let last_r = (cr + r % 2).min(transfer.coarse_nrows - 1);
                let last_c = (cc + c % 2).min(transfer.coarse_ncols - 1);
                let weight = 1.0 / ((last_r - cr + 1) * (last_c - cc + 1)) as f64;
                for i in cr..=last_r {
                    for j in cc..=last_c {
                        reference.add_triplet(r * nc + c, i * transfer.coarse_ncols + j, weight);
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
    fn interior_weights_and_even_endpoint_copy() {
        let transfer = Transfer {
            fine_nrows: 5,
            fine_ncols: 5,
            coarse_nrows: 3,
            coarse_ncols: 3,
        };
        let mut column = Vec::new();
        transfer.for_each_column(4, |row, weight| column.push((row, weight)));
        assert_eq!(
            column,
            vec![
                (6, 0.25),
                (7, 0.5),
                (8, 0.25),
                (11, 0.5),
                (12, 1.0),
                (13, 0.5),
                (16, 0.25),
                (17, 0.5),
                (18, 0.25),
            ]
        );
        let even = Transfer {
            fine_nrows: 4,
            fine_ncols: 4,
            coarse_nrows: 2,
            coarse_ncols: 2,
        };
        let mut corner = Vec::new();
        even.for_each_row(15, |col, weight| corner.push((col, weight)));
        assert_eq!(corner, vec![(3, 1.0)]);
    }

    #[test]
    fn transfers_match_triplets_and_are_transposes() {
        for (nr, nc) in [
            (1, 1),
            (1, 8),
            (9, 1),
            (2, 2),
            (3, 5),
            (8, 8),
            (15, 16),
            (16, 15),
            (2, 19),
            (19, 2),
        ] {
            let transfer = Transfer {
                fine_nrows: nr,
                fine_ncols: nc,
                coarse_nrows: nr.div_ceil(2),
                coarse_ncols: nc.div_ceil(2),
            };
            let reference = reference_matrix(&transfer);
            for (row, entries) in reference.outer_iterator().enumerate() {
                assert!((entries.data().iter().sum::<f64>() - 1.0).abs() < 1e-12);
                if row / nc % 2 == 0 && row % nc % 2 == 0 {
                    assert_eq!(entries.nnz(), 1);
                    assert_eq!(entries.data(), &[1.0]);
                }
            }
            assert!(reference
                .to_csc()
                .outer_iterator()
                .all(|column| column.nnz() <= 9));
            assert_entries_match(&transfer, &reference);
            assert_transfers_and_transpose(&transfer, &reference);
        }
    }
}
