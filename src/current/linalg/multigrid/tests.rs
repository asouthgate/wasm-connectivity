#[cfg(feature = "instrumentation-profile")]
use super::galerkin::GalerkinRow;
use crate::current::memory;
use super::*;
use super::galerkin::galerkin_coarsen_operator;
use crate::current::linalg::smooth::{weighted_jacobi_smooth, symmetric_gauss_seidel_smooth};
use crate::current::linalg::pcg;

// Helper to generate safe mock resistance data for a uniform grid
fn generate_mock_resistance(nrows: usize, ncols: usize) -> Vec<f64> {
    vec![1.0; nrows * ncols]
}

// Build a bilinear MG hierarchy from a resistance raster (nodata -1.0).
fn build_mg(
    resistance: &[f64],
    nrows: usize,
    ncols: usize,
    max_levels: usize,
) -> MgPreconditioner<'static> {
    let filled = crate::raster::fill_nodata(resistance, -1.0);
    let (_cell_to_node, _num_nodes, _edges, mut laplacian) =
        crate::build_circuit_model(&filled, nrows, ncols, -1.0);
    crate::current::circuit::laplacian::regularize_laplacian(&mut laplacian);
    MgPreconditioner::build_from_laplacian(laplacian, nrows, ncols, max_levels)
}

// The tests build the explicit hierarchy, so the fine operator is a CsMat.
fn fine_csmat<'a>(mg: &'a MgPreconditioner<'_>) -> &'a CsMat<f64> {
    match &mg.fine {
        FineOperator::Explicit(m) => m,
        FineOperator::Stencil(_) => panic!("expected an explicit fine operator"),
    }
}

fn num_levels(mg: &MgPreconditioner<'_>) -> usize {
    1 + mg.coarse.len()
}

fn level_n(mg: &MgPreconditioner<'_>, level: usize) -> usize {
    if level == 0 {
        mg.fine.n()
    } else {
        let lvl = &mg.coarse[level - 1];
        lvl.nrows * lvl.ncols
    }
}

fn level_operator<'x>(mg: &'x MgPreconditioner<'_>, level: usize) -> LevelOperator<'x> {
    mg.level_operator(level)
}

fn structural_nnz(op: &dyn Operator) -> usize {
    let mut nnz = 0;
    for row in 0..op.n() {
        op.for_each_entry(row, &mut |_, _| nnz += 1);
    }
    nnz
}

fn materialize(op: &dyn Operator) -> CsMat<f64> {
    let mut triplets = sprs::TriMat::new((op.n(), op.n()));
    for row in 0..op.n() {
        op.for_each_entry(row, &mut |col, val| triplets.add_triplet(row, col, val));
    }
    triplets.to_csr()
}

fn materialize_transfer(transfer: Transfer) -> CsMat<f64> {
    let mut triplets = sprs::TriMat::new((
        transfer.fine_nrows * transfer.fine_ncols,
        transfer.coarse_n(),
    ));
    for col in 0..transfer.coarse_n() {
        transfer.for_each_column::<f64>(col, |row, val| triplets.add_triplet(row, col, val));
    }
    triplets.to_csr()
}

fn assert_operators_match(actual: &dyn Operator, expected: &dyn Operator) {
    assert_eq!(actual.n(), expected.n());
    for row in 0..actual.n() {
        let mut entries = Vec::new();
        let mut reference = Vec::new();
        actual.for_each_entry(row, &mut |col, val| entries.push((col, val)));
        expected.for_each_entry(row, &mut |col, val| reference.push((col, val)));
        entries.sort_by_key(|&(col, _)| col);
        reference.sort_by_key(|&(col, _)| col);
        assert_eq!(entries.len(), reference.len(), "row {row} structure");
        for ((col, val), (ref_col, ref_val)) in entries.iter().zip(&reference) {
            assert_eq!(col, ref_col, "row {row} column");
            assert!(
                (val - ref_val).abs() <= 1e-11 * ref_val.abs().max(1.0),
                "row {row}, col {col}: {val} vs {ref_val}"
            );
        }
    }
    let x: Vec<f64> = (0..actual.n()).map(|i| (i as f64 * 0.3).sin()).collect();
    let mut y = vec![0.0; actual.n()];
    let mut ref_y = y.clone();
    actual.matvec(&x, &mut y);
    expected.matvec(&x, &mut ref_y);
    for (a, b) in y.iter().zip(ref_y) {
        assert!((a - b).abs() <= 1e-10 * b.abs().max(1.0));
    }
    for (a, b) in actual.diag_inv().iter().zip(expected.diag_inv()) {
        assert!((a - b).abs() <= 1e-11 * b.abs().max(1.0));
    }
}

fn reference_galerkin(fine: &dyn Operator, p: &CsMat<f64>, coarse_n: usize) -> CsMat<f64> {
    let p_csc = p.to_csc();

    // Symbolic pass: discover distinct columns in each output row before
    // computing values. These counts give exact-sized CSR allocations, avoiding
    // hash maps and the potentially large intermediate product L_fine * P.
    let mut seen = vec![usize::MAX; coarse_n];
    let mut row_nnz = vec![0usize; coarse_n];
    for i in 0..coarse_n {
        let Some(pcol) = p_csc.outer_view(i) else {
            continue;
        };
        for (a, _) in pcol.iter() {
            fine.for_each_entry(a, &mut |b, _| {
                if let Some(pb) = p.outer_view(b) {
                    for (j, _) in pb.iter() {
                        if seen[j] != i {
                            seen[j] = i;
                            row_nnz[i] += 1;
                        }
                    }
                }
            });
        }
    }

    let mut indptr = vec![0usize; coarse_n + 1];
    for i in 0..coarse_n {
        indptr[i + 1] = indptr[i] + row_nnz[i];
    }
    let nnz = indptr[coarse_n];

    // Numeric pass: accumulate contributions into the allocated CSR slots.
    seen.fill(usize::MAX);
    let mut pos = vec![0usize; coarse_n];
    let mut row_count = vec![0usize; coarse_n];
    let mut cols = vec![0usize; nnz];
    let mut vals = vec![0.0f64; nnz];
    for i in 0..coarse_n {
        let Some(pcol) = p_csc.outer_view(i) else {
            continue;
        };
        for (a, &p_ai) in pcol.iter() {
            fine.for_each_entry(a, &mut |b, l_ab| {
                if let Some(pb) = p.outer_view(b) {
                    for (j, &p_bj) in pb.iter() {
                        let slot = if seen[j] != i {
                            seen[j] = i;
                            let s = indptr[i] + row_count[i];
                            row_count[i] += 1;
                            pos[j] = s;
                            cols[s] = j;
                            vals[s] = 0.0;
                            s
                        } else {
                            pos[j]
                        };
                        vals[slot] += p_ai * l_ab * p_bj;
                    }
                }
            });
        }
    }

    CsMat::new_from_unsorted((coarse_n, coarse_n), indptr, cols, vals).unwrap()
}

#[test]
fn final_smoothing_is_exactly_the_requested_number_of_fine_sweeps() {
    use crate::current::linalg::operator::{StencilOperator, GroundSpec};
    for low_memory in [false, true] {
        let n = 8 * 8;
        let resistance = vec![1.0; n];
        let fine = if low_memory {
            FineOperator::Stencil(StencilOperator::new(8, 8, -9999.0,
                &resistance, GroundSpec::Shunt(vec![1.0; n])))
        } else {
            let (_, _, _, matrix) = crate::build_circuit_model(&vec![1.0; n], 8, 8, -9999.0);
            FineOperator::Explicit(crate::current::circuit::laplacian::add_diagonal(&matrix, &vec![1.0; n]))
        };
        let mg = MgPreconditioner::build_with_options(fine, 8, 8, MgOptions::default());
        let rhs = vec![1.0; n];
        let mut x = vec![0.0; n];
        mg.final_post_smooth(&rhs, &mut x, 0, false, 0);
        assert_eq!(x, vec![0.0; n]);
        let mut expected = x.clone();
        let mut scratch = vec![0.0; n];
        for _ in 0..5 {
            mg.workspaces.borrow()[0].smoother.smooth(mg.fine_operator(),
                &mut expected, &rhs, &mut scratch, mg.workspaces.borrow()[0].omega_override.unwrap_or(mg.omega));
        }
        mg.final_post_smooth(&rhs, &mut x, 5, false, 0);
        assert_eq!(x, expected);
        let _scope = profile::Scope::new(true);
        let mut traced = vec![0.0; n];
        mg.final_post_smooth(&rhs, &mut traced, 5, true, 0);
        assert_eq!(traced, expected);
    }
}

#[test]
fn implicit_galerkin_and_level_two_match_original_coarsening() {
    use crate::current::linalg::operator::{GroundSpec, StencilOperator};
    for (nr, nc) in [(8, 8), (15, 17), (16, 16), (9, 13), (2, 19), (19, 2)] {
        let n = nr * nc;
        let resistance: Vec<f64> = (0..n)
            .map(|i| match i % 13 {
                0 => -9999.0,
                1 => 0.0,
                2 => f64::NAN,
                3 => 1e-3,
                4 => 1e6,
                _ => 1.0 + (i % 9) as f64,
            })
            .collect();
        for ground in 0..3 {
            let spec = match ground {
                0 => GroundSpec::None,
                1 => GroundSpec::Shunt(
                    (0..n)
                        .map(|i| if i % nc == nc - 1 { 2.0 } else { 0.0 })
                        .collect(),
                ),
                _ => GroundSpec::Dirichlet((0..n).map(|i| i % nc == nc - 1).collect()),
            };
            let fine = StencilOperator::new(nr, nc, -9999.0, &resistance, spec);
            let transfer = Transfer {
                fine_nrows: nr,
                fine_ncols: nc,
                coarse_nrows: nr.div_ceil(2),
                coarse_ncols: nc.div_ceil(2),
            };
            let p = materialize_transfer(transfer);
            let fine_matrix = materialize(&fine);
            let mut reference = reference_galerkin(&fine_matrix, &p, transfer.coarse_n());
            let matvec_workspace = RefCell::new(GalerkinMatvecWorkspace::new(fine.n()));
            let raw = GalerkinOperator {
                fine: &fine,
                transfer,
                reg_diag0: 0.0,
                matvec_workspace: &matvec_workspace,
            };
            let (reg, nnz) = raw.setup_metadata();
            assert_eq!(nnz, reference.nnz());
            crate::current::circuit::laplacian::regularize_laplacian(&mut reference);
            let implicit = GalerkinOperator {
                reg_diag0: reg,
                ..raw
            };
            assert_operators_match(&implicit, &reference);
            assert_operators_match(
                &galerkin_coarsen_operator(&fine, transfer),
                &reference_galerkin(&fine_matrix, &p, transfer.coarse_n()),
            );
            if transfer.coarse_nrows >= 2 && transfer.coarse_ncols >= 2 {
                let p1 = Transfer {
                    fine_nrows: nr.div_ceil(2),
                    fine_ncols: nc.div_ceil(2),
                    coarse_nrows: nr.div_ceil(4),
                    coarse_ncols: nc.div_ceil(4),
                };
                let mut actual2 = galerkin_coarsen_operator(&implicit, p1);
                let mut expected2 =
                    reference_galerkin(&reference, &materialize_transfer(p1), p1.coarse_n());
                crate::current::circuit::laplacian::regularize_laplacian(&mut actual2);
                crate::current::circuit::laplacian::regularize_laplacian(&mut expected2);
                assert_operators_match(&actual2, &expected2);
            }
        }
    }
}

#[test]
fn galerkin_preserves_structurally_present_zero_entries() {
    let transfer = Transfer {
        fine_nrows: 8,
        fine_ncols: 8,
        coarse_nrows: 4,
        coarse_ncols: 4,
    };
    let mut diagonal = sprs::TriMat::new((64, 64));
    for i in 0..64 {
        diagonal.add_triplet(i, i, 0.0);
    }
    let fine = diagonal.to_csr::<usize>();
    let matvec_workspace = RefCell::new(GalerkinMatvecWorkspace::new(fine.n()));
    let implicit = GalerkinOperator {
        fine: &fine,
        transfer,
        reg_diag0: 0.0,
        matvec_workspace: &matvec_workspace,
    };
    let expected = reference_galerkin(&fine, &materialize_transfer(transfer), 16);
    assert_operators_match(&implicit, &expected);
    assert_eq!(implicit.setup_metadata().1, expected.nnz());
    assert!(expected.nnz() > 16);
}

struct ProductOnly<'b> {
    matrix: &'b CsMat<f64>,
    calls: std::cell::Cell<usize>,
}

impl Operator for ProductOnly<'_> {
    fn n(&self) -> usize {
        self.matrix.rows()
    }
    fn matvec(&self, x: &[f64], y: &mut [f64]) {
        self.calls.set(self.calls.get() + 1);
        self.matrix.matvec(x, y);
    }
    fn diag_inv(&self) -> Vec<f64> {
        panic!("diagonal must be cached before smoothing")
    }
    fn for_each_entry(&self, _: usize, _: &mut dyn FnMut(usize, f64)) {
        panic!("matvec-based smoothing must not reconstruct coefficients")
    }
}

#[test]
fn manual_omega_is_used_directly_by_both_smoothers_and_final_sweeps() {
    use crate::current::linalg::operator::{GroundSpec, StencilOperator};
    let resistance = vec![1.0; 32 * 32];
    for low_memory in [true, false] {
        let fine = if low_memory {
            FineOperator::Stencil(StencilOperator::new(32, 32, -9999.0, &resistance,
                GroundSpec::Shunt(vec![1.0; resistance.len()])))
        } else {
            let (_, _, _, matrix) = crate::build_circuit_model(&resistance, 32, 32, -9999.0);
            FineOperator::Explicit(crate::current::circuit::laplacian::add_diagonal(&matrix,
                &vec![1.0; resistance.len()]))
        };
        let mg = MgPreconditioner::build_with_options(fine, 32, 32, MgOptions {
            l0nu: 1, l1nu: None, coarsenu: 1, l0omega: Some(0.61), coarseomega: Some(0.83),
            ..MgOptions::default()
        });
        for level in 0..mg.coarse.len() {
            let op = mg.level_operator(level);
            let omega = if level == 0 { 0.61 } else { 0.83 };
            let mut workspaces = mg.workspaces.borrow_mut();
            let current = &mut workspaces[level];
            current.d.resize(op.n(), 1.0);
            current.e.resize(op.n(), 0.0);
            current.d.fill(1.0);
            let mut expected = vec![0.0; op.n()];
            if let Smoother::WeightedJacobi { diag_inv, omega: actual } = &current.smoother {
                assert_eq!(*actual, omega);
                weighted_jacobi_smooth(&op, &mut expected, &current.d, diag_inv,
                    omega, &mut vec![0.0; op.n()]);
            } else {
                symmetric_gauss_seidel_smooth(&op, &mut expected, &current.d, omega);
            }
            mg.smooth_phase::<false>(&mut current.as_cycle(), &op, level, true);
            assert_eq!(current.e, expected);
            if level == 0 {
                drop(workspaces);
                let mut x = vec![0.0; op.n()];
                mg.final_post_smooth(&vec![1.0; op.n()], &mut x, 1, false, 0);
                assert_eq!(x, expected);
            }
        }
    }
}

#[test]
fn fine_and_coarse_schedule_covers_all_levels() {
    use crate::current::linalg::operator::{GroundSpec, StencilOperator};
    let resistance = vec![1.0; 128 * 128];
    for (l0nu, l1nu, coarsenu) in [(3, None, 10), (2, None, 9), (2, Some(7), 9)] {
        let fine = FineOperator::Stencil(StencilOperator::new(
            128, 128, -9999.0, &resistance,
            GroundSpec::Shunt(vec![1.0; resistance.len()]),
        ));
        let mg = MgPreconditioner::build_with_options(fine, 128, 128,
            MgOptions { l0nu, l1nu, coarsenu, cholesky_size: Some(1), ..MgOptions::default() });
        let counts: Vec<_> = mg.workspaces.borrow().iter().map(|w| w.nu).collect();
        assert_eq!(counts, vec![l0nu, l1nu.unwrap_or(l0nu), coarsenu, coarsenu, coarsenu, coarsenu, coarsenu, 0]);
    }
}

#[test]
fn fine_coarse_sweeps_preserve_preconditioner_symmetry() {
    use crate::current::linalg::operator::{GroundSpec, StencilOperator};
    let n = 32 * 32;
    let resistance = vec![1.0; n];
    let x: Vec<f64> = (0..n).map(|i| (i as f64 * 0.1).sin()).collect();
    let y: Vec<f64> = (0..n).map(|i| (i as f64 * 0.2).cos()).collect();
    for low_memory in [false, true] {
        for (l0nu, coarsenu) in [(2, 2), (3, 10), (1, 8), (5, 12)] {
            let fine = if low_memory {
                FineOperator::Stencil(StencilOperator::new(
                    32, 32, -9999.0, &resistance, GroundSpec::Shunt(vec![1.0; n]),
                ))
            } else {
                let (_, _, _, matrix) = crate::build_circuit_model(&resistance, 32, 32, -9999.0);
                FineOperator::Explicit(crate::current::circuit::laplacian::add_diagonal(&matrix, &vec![1.0; n]))
            };
            let mg = MgPreconditioner::build_with_options(fine, 32, 32,
                MgOptions { l0nu, l1nu: None, coarsenu, ..MgOptions::default() });
            let counts: Vec<usize> = mg.workspaces.borrow().iter().map(|w| w.nu).collect();
            assert_eq!(counts, vec![l0nu, l0nu, coarsenu, 0]);
            let mut mx = Vec::new();
            let mut my = Vec::new();
            mg.apply(&x, &mut mx);
            mg.apply(&y, &mut my);
            let x_my: f64 = x.iter().zip(&my).map(|(a, b)| a * b).sum();
            let y_mx: f64 = y.iter().zip(&mx).map(|(a, b)| a * b).sum();
            assert!((x_my - y_mx).abs() < 1e-9 * x_my.abs().max(y_mx.abs()).max(1.0));
            assert!(x.iter().zip(&mx).map(|(a, b)| a * b).sum::<f64>() > 0.0);
        }
    }
}

#[test]
fn weighted_jacobi_updates_simultaneously_using_only_matvec() {
    let mut entries = sprs::TriMat::<f64>::new((3, 3));
    for i in 0..3 {
        entries.add_triplet(i, i, 2.0);
    }
    for i in 0..2 {
        entries.add_triplet(i, i + 1, -1.0);
        entries.add_triplet(i + 1, i, -1.0);
    }
    let matrix = entries.to_csr::<usize>();
    let op = ProductOnly {
        matrix: &matrix,
        calls: std::cell::Cell::new(0),
    };
    let mut e = vec![1.0, 2.0, 4.0];
    let mut scratch = vec![99.0; 3];
    let ptr = scratch.as_ptr();
    weighted_jacobi_smooth(&op, &mut e, &[3.0, -2.0, 1.0], &[0.5; 3], 0.5, &mut scratch);
    assert_eq!(e, vec![1.75, 1.75, 2.75]);
    weighted_jacobi_smooth(&op, &mut e, &[3.0, -2.0, 1.0], &[0.5; 3], 0.5, &mut scratch);
    assert_eq!(e, vec![2.0625, 1.5, 2.0625]);
    assert_eq!(op.calls.get(), 2);
    assert_eq!(scratch.as_ptr(), ptr);
}

#[test]
fn jacobi_damping_is_capped_for_large_normalized_eigenvalues() {
    let mut entries = sprs::TriMat::<f64>::new((4, 4));
    for i in 0..4 {
        for j in 0..4 {
            entries.add_triplet(i, j, if i == j { 1.0 } else { 0.9 });
        }
    }
    let matrix = entries.to_csr::<usize>();
    let smoother = Smoother::weighted_jacobi(&matrix, 0.67);
    let Smoother::WeightedJacobi { diag_inv, omega } = &smoother else {
        panic!("expected Jacobi")
    };
    assert_eq!(diag_inv, &vec![1.0; 4]);
    assert!((*omega - 1.95 / 3.7).abs() < 1e-14);
    let mut e = vec![1.0; 4];
    smoother.smooth(&matrix, &mut e, &[0.0; 4], &mut [0.0; 4], 0.67);
    for value in e {
        assert!((value + 0.95).abs() < 1e-14);
    }
}

#[test]
fn all_jacobi_levels_request_full_weight_and_damp_quiet_cells() {
    use crate::current::linalg::operator::{GroundSpec, StencilOperator};
    let resistance = vec![1e4; 32 * 32];
    let op = StencilOperator::new(32, 32, -9999.0, &resistance,
        GroundSpec::Shunt(vec![1.0; resistance.len()]));
    let mg = MgPreconditioner::build_with_options(FineOperator::Stencil(op), 32, 32,
        MgOptions { l0omega: None, coarseomega: None, ..MgOptions::default() });
    let workspaces = mg.workspaces.borrow();
    let Smoother::WeightedJacobi { omega, .. } = workspaces[0].smoother else {
        panic!("expected fine Jacobi smoother")
    };
    assert_eq!(omega, 1.0);
    let fine = mg.level_operator(0);
    let mut error = vec![1.0; resistance.len()];
    workspaces[0].smoother.smooth(&fine, &mut error, &vec![0.0; resistance.len()],
        &mut vec![0.0; resistance.len()], 0.67);
    // A constant error has no branch flux; the shunt dominates this
    // interior equation, so full Jacobi should almost remove it in one sweep.
    assert!(error[16 * 32 + 16] > 0.0 && error[16 * 32 + 16] < 0.001);
    for level in 1..workspaces.len() - 1 {
        let op = mg.level_operator(level);
        let Smoother::WeightedJacobi { omega: expected, .. } = Smoother::weighted_jacobi(&op, 1.0) else {
            unreachable!()
        };
        let Smoother::WeightedJacobi { omega: actual, .. } = workspaces[level].smoother else {
            panic!("expected coarse Jacobi smoother")
        };
        assert_eq!(actual, expected);
    }
}

#[test]
fn jacobi_damping_uses_symmetric_scaling_for_unequal_diagonals() {
    let mut entries = sprs::TriMat::<f64>::new((2, 2));
    for (i, j, value) in [(0, 0, 1.0), (0, 1, 9.0), (1, 0, 9.0), (1, 1, 100.0)] {
        entries.add_triplet(i, j, value);
    }
    let matrix = entries.to_csr::<usize>();
    let smoother = Smoother::weighted_jacobi(&matrix, 0.67);
    let Smoother::WeightedJacobi { omega, .. } = &smoother else {
        panic!("expected Jacobi")
    };
    // The original bound is 10; the symmetric bound and largest
    // eigenvalue are both 1.9, so the requested weight is safe.
    assert_eq!(*omega, 0.67);
    let mut e = vec![1.0, 0.1];
    smoother.smooth(&matrix, &mut e, &[0.0; 2], &mut [0.0; 2], 0.67);
    assert!((e[0] - (1.0 - 0.67 * 1.9)).abs() < 1e-14);
    assert!((e[1] - 0.1 * (1.0 - 0.67 * 1.9)).abs() < 1e-14);
}

#[test]
fn symmetric_jacobi_bound_still_caps_unequal_diagonal_systems() {
    let scales = [1.0, 10.0, 100.0, 1000.0];
    let mut entries = sprs::TriMat::<f64>::new((4, 4));
    for i in 0..4 {
        for j in 0..4 {
            entries.add_triplet(i, j, scales[i] * scales[j]
                * if i == j { 1.0 } else { 0.9 });
        }
    }
    let matrix = entries.to_csr::<usize>();
    let smoother = Smoother::weighted_jacobi(&matrix, 0.67);
    let Smoother::WeightedJacobi { omega, .. } = &smoother else {
        panic!("expected Jacobi")
    };
    assert!((*omega - 1.95 / 3.7).abs() < 1e-14);
    let mut e: Vec<f64> = scales.iter().map(|scale| 1.0 / scale).collect();
    smoother.smooth(&matrix, &mut e, &[0.0; 4], &mut [0.0; 4], 0.67);
    for (value, scale) in e.iter().zip(scales) {
        assert!((value * scale + 0.95).abs() < 1e-14);
    }
}

#[test]
fn composed_galerkin_applies_fine_operator_once_and_reuses_scratch() {
    let nr = 8;
    let nc = 8;
    let resistance = vec![1.0; nr * nc];
    let (_, _, _, matrix) = crate::build_circuit_model(&resistance, nr, nc, -9999.0);
    let transfer = Transfer {
        fine_nrows: nr,
        fine_ncols: nc,
        coarse_nrows: 4,
        coarse_ncols: 4,
    };
    let mut expected = reference_galerkin(&matrix, &materialize_transfer(transfer), 16);
    let reg = 0.123;
    expected.get_mut(0, 0).map(|diag| *diag += reg).unwrap();
    let fine = ProductOnly {
        matrix: &matrix,
        calls: std::cell::Cell::new(0),
    };
    let scratch = RefCell::new(GalerkinMatvecWorkspace::new(64));
    let ptrs = {
        let w = scratch.borrow();
        (w.prolonged.as_ptr(), w.applied.as_ptr())
    };
    let op = GalerkinOperator {
        fine: &fine,
        transfer,
        reg_diag0: reg,
        matvec_workspace: &scratch,
    };
    let x: Vec<f64> = (0..16).map(|i| (i as f64 * 0.3).sin()).collect();
    let mut actual = vec![77.0; 16];
    let mut reference = vec![0.0; 16];
    expected.matvec(&x, &mut reference);
    for _ in 0..2 {
        op.matvec(&x, &mut actual);
        for (a, b) in actual.iter().zip(&reference) {
            assert!((a - b).abs() < 1e-12);
        }
    }
    op.matvec(&[0.0; 16], &mut actual);
    assert_eq!(actual, vec![0.0; 16]);
    assert_eq!(fine.calls.get(), 3);
    let w = scratch.borrow();
    assert_eq!((w.prolonged.as_ptr(), w.applied.as_ptr()), ptrs);
}

#[test]
fn small_grids_and_cholesky_size_reuse_workspaces() {
    use crate::current::linalg::operator::{GroundSpec, StencilOperator};
    for (nr, nc, depth, target, expected) in [
        (1, 1, 8, None, 1),
        (1, 1, 8, Some(1), 1),
        (1, 9, 8, Some(1), 1),
        (9, 1, 8, Some(1), 1),
        (3, 5, 8, None, 15),
        (7, 7, 8, None, 16),
        (16, 16, 1, None, 256),
        (3, 64, 8, None, 192),
        (16, 16, 8, Some(64), 64),
        (16, 16, 8, Some(4), 4),
        (16, 16, 8, Some(1), 1),
        (16, 16, 2, Some(1), 64),
    ] {
        let resistance = vec![1.0; nr * nc];
        let (_, _, _, laplacian) = crate::build_circuit_model(&resistance, nr, nc, -9999.0);
        let laplacian =
            crate::current::circuit::laplacian::add_diagonal(&laplacian, &vec![1.0; nr * nc]);
        let options = MgOptions {
            l0nu: 3,
            l1nu: None,
            coarsenu: 10,
            l0omega: None,
            coarseomega: None,
            verbose: false,
            profile: false,
            l0postnu: 0,
            max_levels: depth,
            cholesky_size: target,
        };
        memory::reset();
        let explicit = MgPreconditioner::build_with_options(
            FineOperator::Explicit(laplacian),
            nr,
            nc,
            options,
        );
        #[cfg(feature = "instrumentation-profile")]
        {
            let profile = memory::take_profile().unwrap();
            assert!(profile.hierarchy[0].laplacian_bytes > 0);
            for level in &profile.hierarchy {
                let terminal = level.level == profile.hierarchy.len() - 1;
                assert_eq!(
                    level.smoother_repr,
                    if terminal {
                        "none"
                    } else {
                        "weighted-jacobi"
                    }
                );
                assert_eq!(level.smoother_diag_bytes, if terminal { 0 } else { memory::vec_f64_bytes(level.nodes) });
                assert_eq!(level.operator_matvec_scratch_bytes, 0);
            }
            for level in &profile.hierarchy[1..] {
                // CSR output arrays are retained in the Laplacian, not scratch.
                assert_eq!(
                    level.laplacian_bytes,
                    memory::vec_usize_bytes(level.nodes + 1)
                        + memory::vec_usize_bytes(level.nnz)
                        + memory::vec_f64_bytes(level.nnz)
                );
                assert_eq!(level.prolongation_p_bytes, 0);
                assert_eq!(level.galerkin_scratch.p_triplets_clone_bytes, 0);
                assert_eq!(level.galerkin_scratch.p_csr_bytes, 0);
                assert_eq!(level.galerkin_scratch.p_csc_bytes, 0);
            }
        }
        memory::reset();
        let stencil = StencilOperator::new(
            nr,
            nc,
            -9999.0,
            &resistance,
            GroundSpec::Shunt(vec![1.0; nr * nc]),
        );
        let stencil = MgPreconditioner::build_with_options(
            FineOperator::Stencil(stencil),
            nr,
            nc,
            options,
        );
        assert_eq!(level_n(&explicit, explicit.coarse.len()), expected);
        assert_eq!(level_n(&stencil, stencil.coarse.len()), expected);
        #[cfg(feature = "instrumentation-profile")]
        {
            let profile = memory::take_profile().unwrap();
            assert_eq!(profile.coarsest_nodes, expected);
            assert_eq!(profile.requested_cholesky_size, target);
            assert_eq!(profile.hierarchy[0].laplacian_bytes, 0);
            assert_eq!(profile.ground_storage_bytes, memory::vec_f64_bytes(nr * nc));
            assert_eq!(profile.hierarchy[0].operator_repr, "stencil");
            for level in &profile.hierarchy {
                let terminal = level.level == profile.hierarchy.len() - 1;
                assert_eq!(
                    level.smoother_repr,
                    if terminal { "none" } else { "weighted-jacobi" }
                );
                assert_eq!(
                    level.smoother_diag_bytes,
                    if terminal {
                        0
                    } else {
                        memory::vec_f64_bytes(level.nodes)
                    }
                );
                assert_eq!(
                    level.operator_matvec_scratch_bytes,
                    if level.level == 1 {
                        memory::vec_f64_bytes(2 * nr * nc)
                    } else {
                        0
                    }
                );
            }
            for level in &profile.hierarchy[1..] {
                assert_eq!(level.prolongation_p_bytes, 0);
                assert_eq!(level.galerkin_scratch.p_triplets_clone_bytes, 0);
                assert_eq!(level.galerkin_scratch.p_csr_bytes, 0);
                assert_eq!(level.galerkin_scratch.p_csc_bytes, 0);
                if level.level == 1 {
                    assert_eq!(level.operator_repr, "galerkin-stencil");
                    assert_eq!(level.laplacian_bytes, 0);
                    assert_eq!(level.galerkin_scratch.symbolic_seen_bytes, 0);
                    assert_eq!(level.galerkin_scratch.numeric_pos_bytes, 0);
                    assert_eq!(
                        level.galerkin_scratch.row_accumulator_bytes,
                        std::mem::size_of::<GalerkinRow>() as u64
                    );
                } else {
                    assert_eq!(level.operator_repr, "explicit");
                    assert!(level.laplacian_bytes > 0);
                }
            }
        }
        let source: Vec<f64> = (0..nr * nc).map(|i| 1.0 + (i as f64).sin()).collect();
        let original_source = source.clone();
        let (mut first, mut again, mut reference) = (Vec::new(), Vec::new(), Vec::new());
        stencil.apply(&source, &mut first);
        let output_storage = first.as_ptr();
        stencil.apply(&source, &mut first);
        assert_eq!(first.as_ptr(), output_storage);
        assert_eq!(source, original_source);
        stencil.apply(&vec![0.0; nr * nc], &mut again);
        assert!(again.iter().all(|&value| value == 0.0));
        stencil.apply(&source, &mut again);
        explicit.apply(&source, &mut reference);
        assert_eq!(
            first, again,
            "scratch reuse must not retain previous solutions"
        );
        for (&a, &b) in first.iter().zip(&reference) {
            assert!((a - b).abs() < 1e-8);
        }
        for mg in [&explicit, &stencil] {
            let workspaces = mg.workspaces.borrow();
            assert_eq!(workspaces[0].e.capacity(), 0);
            assert_eq!(workspaces[0].d.capacity(), 0);
        }
    }
}

#[test]
fn test_stencil_matches_explicit_mg() {
    use crate::current::linalg::operator::{GroundSpec, StencilOperator};

    let nrows = 16;
    let ncols = 16;
    let n = nrows * ncols;
    let nodata = crate::NODATA_SENTINEL;

    let mut resistance = vec![1.0; n];
    for r in 0..nrows {
        for c in 0..ncols {
            let i = r * ncols + c;
            if c % 4 == 0 {
                resistance[i] = 0.001;
            }
            if r % 5 == 3 {
                resistance[i] = 1000.0;
            }
        }
    }
    resistance[5 * ncols + 5] = nodata;

    let mut gnd = vec![0.0; n];
    gnd[n - 1] = 1.0;
    gnd[n - ncols] = 2.0;

    let filled = crate::raster::fill_nodata(&resistance, nodata);
    let (_ctn, _nn, _e, lap) = crate::build_circuit_model(&filled, nrows, ncols, nodata);

    let shunt: Vec<f64> = gnd.iter().map(|&g| if g > 0.0 { g } else { 0.0 }).collect();
    let mask: Vec<bool> = gnd.iter().map(|&g| g > 0.0).collect();
    let gns: Vec<usize> = (0..n).filter(|&i| mask[i]).collect();

    let x: Vec<f64> = (0..n).map(|i| (i as f64 * 0.13).sin()).collect();

    let check = |mg_e: &MgPreconditioner, mg_s: &MgPreconditioner, tag: &str| {
        assert_eq!(
            structural_nnz(&mg_e.level_operator(1)),
            structural_nnz(&mg_s.level_operator(1)),
            "{tag}: coarse level-1 nnz differs"
        );
        let mut ze = vec![0.0; n];
        let mut zs = vec![0.0; n];
        mg_e.apply(&x, &mut ze);
        mg_s.apply(&x, &mut zs);
        for i in 0..n {
            assert!(
                (ze[i] - zs[i]).abs() < 1e-6,
                "{tag}: MG stencil vs explicit mismatch at {}: {} vs {}",
                i,
                ze[i],
                zs[i]
            );
        }
    };

    // No ground
    {
        let mut ungrounded = lap.clone();
        crate::current::circuit::laplacian::regularize_laplacian(&mut ungrounded);
        let mg_e = MgPreconditioner::build_from_laplacian(ungrounded, nrows, ncols, 5);
        let st = StencilOperator::new(nrows, ncols, nodata, &resistance, GroundSpec::None);
        let mg_s =
            MgPreconditioner::build_from_operator(FineOperator::Stencil(st), nrows, ncols, 5);
        check(&mg_e, &mg_s, "none");
    }

    // Shunt ground
    {
        let a = crate::current::circuit::laplacian::add_diagonal(&lap, &shunt);
        let mg_e = MgPreconditioner::build_from_laplacian(a, nrows, ncols, 5);
        let st = StencilOperator::new(
            nrows,
            ncols,
            nodata,
            &resistance,
            GroundSpec::Shunt(shunt.clone()),
        );
        let mg_s =
            MgPreconditioner::build_from_operator(FineOperator::Stencil(st), nrows, ncols, 5);
        check(&mg_e, &mg_s, "shunt");
    }

    // Dirichlet ground
    {
        let a = crate::current::apply_dirichlet_ground_lap(&lap, &gns);
        let mg_e = MgPreconditioner::build_from_laplacian(a, nrows, ncols, 5);
        let st = StencilOperator::new(
            nrows,
            ncols,
            nodata,
            &resistance,
            GroundSpec::Dirichlet(mask.clone()),
        );
        let mg_s =
            MgPreconditioner::build_from_operator(FineOperator::Stencil(st), nrows, ncols, 5);
        check(&mg_e, &mg_s, "dirichlet");
    }
}

#[test]
fn matrix_free_preconditioner_is_symmetric_and_positive() {
    use crate::current::linalg::operator::{GroundSpec, StencilOperator};
    for (nr, nc) in [(16, 16), (17, 19)] {
        let n = nr * nc;
        let resistance: Vec<f64> = (0..n).map(|i| 0.5 + (i % 7) as f64).collect();
        let x: Vec<f64> = (0..n).map(|i| (i as f64 * 0.1).sin()).collect();
        let y: Vec<f64> = (0..n).map(|i| (i as f64 * 0.2).cos()).collect();
        for mode in 0..3 {
            let spec = match mode {
                0 => GroundSpec::None,
                1 => GroundSpec::Shunt(
                    (0..n)
                        .map(|i| if i % nc == nc - 1 { 1.0 } else { 0.0 })
                        .collect(),
                ),
                _ => GroundSpec::Dirichlet((0..n).map(|i| i % nc == nc - 1).collect()),
            };
            let fine = StencilOperator::new(nr, nc, -9999.0, &resistance, spec);
            let mg =
                MgPreconditioner::build_from_operator(FineOperator::Stencil(fine), nr, nc, 8);
            let mut mx = Vec::new();
            let mut my = Vec::new();
            mg.apply(&x, &mut mx);
            mg.apply(&y, &mut my);
            let x_my: f64 = x.iter().zip(&my).map(|(a, b)| a * b).sum();
            let y_mx: f64 = y.iter().zip(&mx).map(|(a, b)| a * b).sum();
            assert!((x_my - y_mx).abs() < 1e-9 * x_my.abs().max(y_mx.abs()).max(1.0));
            assert!(x.iter().zip(&mx).map(|(a, b)| a * b).sum::<f64>() > 0.0);
            for level in 0..num_levels(&mg) {
                let op = mg.level_operator(level);
                for diagonal in op.diag_inv() {
                    assert!(diagonal > 0.0);
                }
                let mut error: Vec<f64> = (0..op.n())
                    .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
                    .collect();
                let before: f64 = error.iter().map(|x| x * x).sum();
                for _ in 0..3 {
                    mg.workspaces.borrow()[level].smoother.smooth(
                        &op,
                        &mut error,
                        &vec![0.0; op.n()],
                        &mut vec![0.0; op.n()],
                        mg.omega,
                    );
                }
                assert!(error.iter().map(|x| x * x).sum::<f64>() < before);
            }
        }
    }
}

#[test]
fn test_multigrid_preconditioner_symmetry() {
    let nrows = 15;
    let ncols = 15;
    let n = nrows * ncols;

    // 1. Build using your actual `.build` method
    let resistance = generate_mock_resistance(nrows, ncols);
    let mg = build_mg(&resistance, nrows, ncols, 3);

    // 2. Generate two distinct structural vectors using predictable trigonometric waves
    let x: Vec<f64> = (0..n).map(|i| (i as f64 * 0.1).sin()).collect();
    let y: Vec<f64> = (0..n).map(|i| (i as f64 * 0.2).cos()).collect();

    let mut mx = vec![0.0; n];
    let mut my = vec![0.0; n];

    let mut ax = vec![0.0; n];
    let mut ay = vec![0.0; n];
    pcg::mat_vec_mul_into(fine_csmat(&mg), &x, &mut ax);
    pcg::mat_vec_mul_into(fine_csmat(&mg), &y, &mut ay);
    let dot_x_ay: f64 = x.iter().zip(ay.iter()).map(|(a, b)| a * b).sum();
    let dot_y_ax: f64 = y.iter().zip(ax.iter()).map(|(a, b)| a * b).sum();
    assert!(
        (dot_x_ay - dot_y_ax).abs() < 1e-7,
        "Fine matrix A is asymmetric!"
    );

    mg.apply(&x, &mut mx);
    mg.apply(&y, &mut my);

    // 3. Compute dot products: x · M(y) vs y · M(x)
    let dot_x_my: f64 = x.iter().zip(my.iter()).map(|(a, b)| a * b).sum();
    let dot_y_mx: f64 = y.iter().zip(mx.iter()).map(|(a, b)| a * b).sum();

    // Operators must be symmetric up to floating-point drift
    assert!(
        (dot_x_my - dot_y_mx).abs() < 1e-6,
        "MG Preconditioner is asymmetric! x*M(y) = {}, y*M(x) = {}",
        dot_x_my,
        dot_y_mx
    );
}

#[test]
fn test_mg_preconditioned_cg_converges() {
    let nrows = 64;
    let ncols = 64;
    let n = nrows * ncols;

    let resistance = generate_mock_resistance(nrows, ncols);
    let mg = build_mg(&resistance, nrows, ncols, 7);

    let b_raw: Vec<f64> = (0..n)
        .map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5)
        .collect();
    let mean: f64 = b_raw.iter().sum::<f64>() / n as f64;
    let b: Vec<f64> = b_raw.iter().map(|v| v - mean).collect();

    let mut x = vec![0.0; n];
    let mut r = b.clone();
    let b_norm = b.iter().map(|v| v * v).sum::<f64>().sqrt();

    let mut residuals = Vec::new();

    for _iter in 0..50 {
        let r_norm = r.iter().map(|v| v * v).sum::<f64>().sqrt();
        residuals.push(r_norm);

        let mut z = vec![0.0; n];
        mg.apply(&r, &mut z);

        let mut az = vec![0.0; n];
        pcg::mat_vec_mul_into(fine_csmat(&mg), &z, &mut az);
        let z_az: f64 = z.iter().zip(az.iter()).map(|(a, b)| a * b).sum();
        if z_az.abs() < 1e-30 {
            break;
        }

        let alpha = {
            let rz: f64 = r.iter().zip(z.iter()).map(|(a, b)| a * b).sum();
            rz / z_az
        };

        for i in 0..n {
            x[i] += alpha * z[i];
            r[i] -= alpha * az[i];
        }

        if r_norm / b_norm < 1e-6 {
            break;
        }
    }

    let r0 = residuals[0];
    let r_last = residuals[residuals.len() - 1];
    eprintln!(
        "Large uniform {}x{}: {} iters, ||r|| went from {:.6e} to {:.6e} (ratio {:.4e})",
        nrows,
        ncols,
        residuals.len(),
        r0,
        r_last,
        r_last / r0
    );
    assert!(
        r_last < r0 * 1e-2,
        "MG-preconditioned CG did not converge on large uniform grid: ||r|| went from {:.6e} to {:.6e} in {} iters",
        r0, r_last, residuals.len()
    );
}

#[test]
fn test_laplacian_diagonal_positive() {
    let nrows = 15;
    let ncols = 15;
    let resistance = generate_mock_resistance(nrows, ncols);
    let mg = build_mg(&resistance, nrows, ncols, 4);

    for level in 0..num_levels(&mg) {
        let n = level_n(&mg, level);
        let op = level_operator(&mg, level);
        for row in 0..n {
            op.for_each_entry(row, &mut |col, val| {
                if col == row {
                    assert!(
                        val > 0.0,
                        "Level {}: diagonal[{}] = {:.6e} (must be positive)",
                        level,
                        row,
                        val
                    );
                    let diag_inv = if val.abs() > 1e-15 {
                        1.0 / val.abs()
                    } else {
                        0.0
                    };
                    assert!(
                        diag_inv > 0.0,
                        "Level {}: diag_inv[{}] = {:.6e} (must be positive)",
                        level,
                        row,
                        diag_inv
                    );
                }
            });
        }
    }
}

#[test]
fn test_preconditioner_positive_definite() {
    let nrows = 15;
    let ncols = 15;
    let n = nrows * ncols;
    let resistance = generate_mock_resistance(nrows, ncols);
    let mg = build_mg(&resistance, nrows, ncols, 4);

    let z: Vec<f64> = (0..n)
        .map(|i| (i as f64 * 0.7 + 1.3).sin() * 0.5 + 0.3)
        .collect();
    let mut mz = vec![0.0; n];
    mg.apply(&z, &mut mz);

    let z_mz: f64 = z.iter().zip(mz.iter()).map(|(a, b)| a * b).sum();
    assert!(
        z_mz > 0.0,
        "Preconditioner is not positive definite: zᵀ·M⁻¹·z = {:.6e}",
        z_mz
    );

    let z2: Vec<f64> = (0..n)
        .map(|i| (i as f64 * 1.1 + 2.7).cos() * 0.8 - 0.2)
        .collect();
    let mut mz2 = vec![0.0; n];
    mg.apply(&z2, &mut mz2);
    let z2_mz2: f64 = z2.iter().zip(mz2.iter()).map(|(a, b)| a * b).sum();
    assert!(
        z2_mz2 > 0.0,
        "Preconditioner is not positive definite (vector 2): zᵀ·M⁻¹·z = {:.6e}",
        z2_mz2
    );
}

#[test]
fn test_preconditioned_search_direction_aligns_with_residual() {
    let nrows = 15;
    let ncols = 15;
    let n = nrows * ncols;
    let resistance = generate_mock_resistance(nrows, ncols);
    let mg = build_mg(&resistance, nrows, ncols, 4);

    let b_raw: Vec<f64> = (0..n)
        .map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5)
        .collect();
    let mean: f64 = b_raw.iter().sum::<f64>() / n as f64;
    let b: Vec<f64> = b_raw.iter().map(|v| v - mean).collect();

    let mut z = vec![0.0; n];
    mg.apply(&b, &mut z);

    let r_z: f64 = b.iter().zip(z.iter()).map(|(a, b)| a * b).sum();
    assert!(
        r_z > 0.0,
        "Preconditioned residual is anti-aligned with residual: rᵀ·z = {:.6e} (must be > 0)",
        r_z
    );
}

#[test]
fn test_coarse_solve_correctness() {
    let nrows = 15;
    let ncols = 15;
    let resistance = generate_mock_resistance(nrows, ncols);
    let mg = build_mg(&resistance, nrows, ncols, 4);

    let lvl = mg.coarse.last().unwrap();
    let n = lvl.nrows * lvl.ncols;

    assert!(
        lvl.cholesky_l.is_some(),
        "Coarsest level should have Cholesky factorization"
    );

    let b: Vec<f64> = (0..n)
        .map(|i| ((i as f64 * 0.5 + 0.3).sin() + 1.0) * 0.5)
        .collect();
    let l = lvl.cholesky_l.as_ref().unwrap();
    let z = crate::current::linalg::cholesky::cholesky_solve(l, &b, n);

    let mut az = vec![0.0; n];
    lvl.operator(&mg.fine).matvec(&z, &mut az);

    let mut max_err = 0.0;
    for i in 0..n {
        let err = (az[i] - b[i]).abs();
        if err > max_err {
            max_err = err;
        }
    }
    assert!(
        max_err < 1e-4,
        "Coarsest level solve error too large: max|Az - b| = {:.6e}",
        max_err
    );
}

#[test]
fn test_smoother_converges_on_all_levels() {
    let nrows = 15;
    let ncols = 15;
    let resistance = generate_mock_resistance(nrows, ncols);
    let mg = build_mg(&resistance, nrows, ncols, 4);

    for level in 0..num_levels(&mg) {
        let n = level_n(&mg, level);
        let b = vec![0.0; n];
        let mut x: Vec<f64> = (0..n)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();

        let initial_norm = x.iter().map(|v| v * v).sum::<f64>().sqrt();
        for _ in 0..3 {
            symmetric_gauss_seidel_smooth(&level_operator(&mg, level), &mut x, &b, mg.omega);
        }
        let final_norm = x.iter().map(|v| v * v).sum::<f64>().sqrt();

        assert!(
            final_norm < initial_norm,
            "Level {}: smoother did not reduce error norm (initial={:.6e}, final={:.6e})",
            level,
            initial_norm,
            final_norm
        );
    }
}

#[test]
fn test_mg_preconditioner_on_full_grid() {
    let nrows = 8;
    let ncols = 8;
    let n = nrows * ncols;

    let resistance = generate_mock_resistance(nrows, ncols);
    let (_cell_to_node, num_nodes, _edges, _full_lap) =
        crate::build_circuit_model(&resistance, nrows, ncols, -1.0);
    assert_eq!(num_nodes, n);

    let mg = build_mg(&resistance, nrows, ncols, 4);

    let b_raw: Vec<f64> = (0..n)
        .map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5)
        .collect();
    let mean: f64 = b_raw.iter().sum::<f64>() / n as f64;
    let b: Vec<f64> = b_raw.iter().map(|v| v - mean).collect();

    let mut z = vec![0.0; n];
    mg.apply(&b, &mut z);

    let mut az = vec![0.0; n];
    pcg::mat_vec_mul_into(fine_csmat(&mg), &z, &mut az);
    let z_az: f64 = z.iter().zip(az.iter()).map(|(a, b)| a * b).sum();
    let rz: f64 = b.iter().zip(z.iter()).map(|(a, b)| a * b).sum();

    let b_norm = b.iter().map(|v| v * v).sum::<f64>().sqrt();
    eprintln!("Full-grid test: n={}", n);
    eprintln!(
        "  ||b|| = {:.6e}, r·z = {:.6e}, z·Az = {:.6e}",
        b_norm, rz, z_az
    );

    assert!(
        z_az > 0.0,
        "MG preconditioner on full grid: z·Az = {:.6e} (must be > 0)",
        z_az
    );
    assert!(
        rz > 0.0,
        "MG preconditioner on full grid: r·z = {:.6e} (must be > 0)",
        rz
    );
}

#[test]
fn test_diag_inv_range_across_levels() {
    let nrows = 32;
    let ncols = 32;
    let resistance = generate_mock_resistance(nrows, ncols);
    let mg = build_mg(&resistance, nrows, ncols, 6);

    for level in 0..num_levels(&mg) {
        let op = level_operator(&mg, level);
        let diag_inv = op.diag_inv();
        let mut min_inv = f64::MAX;
        let mut max_inv = 0.0;
        for &d in &diag_inv {
            if d < min_inv {
                min_inv = d;
            }
            if d > max_inv {
                max_inv = d;
            }
        }
        eprintln!(
            "Level {}: diag_inv range = [{:.6e}, {:.6e}] (ratio = {:.2e})",
            level,
            min_inv,
            max_inv,
            max_inv / min_inv.max(1e-30)
        );

        assert!(
            min_inv > 1e-15,
            "Level {}: diag_inv too small: min = {:.6e}",
            level,
            min_inv
        );
        assert!(
            max_inv < 1e10,
            "Level {}: diag_inv too large: max = {:.6e}",
            level,
            max_inv
        );
    }
}

#[test]
fn test_coarse_cholesky_accuracy() {
    let nrows = 32;
    let ncols = 32;
    let resistance = generate_mock_resistance(nrows, ncols);
    let mg = build_mg(&resistance, nrows, ncols, 6);

    let lvl = mg.coarse.last().unwrap();
    let n = lvl.nrows * lvl.ncols;

    assert!(
        lvl.cholesky_l.is_some(),
        "Coarsest level should have Cholesky factorization"
    );

    let l = lvl.cholesky_l.as_ref().unwrap();
    let b: Vec<f64> = (0..n)
        .map(|i| ((i as f64 * 0.5 + 0.3).sin() + 1.0) * 0.5)
        .collect();
    let x = crate::current::linalg::cholesky::cholesky_solve(l, &b, n);

    let mut ax = vec![0.0; n];
    lvl.operator(&mg.fine).matvec(&x, &mut ax);

    let mut max_err = 0.0;
    for i in 0..n {
        let err = (ax[i] - b[i]).abs();
        if err > max_err {
            max_err = err;
        }
    }
    assert!(
        max_err < 1e-6,
        "Coarsest level Cholesky solve error: max|Ax - b| = {:.6e}",
        max_err
    );
}

#[test]
fn test_mg_preconditioned_cg_variable_coefficients() {
    let nrows = 32;
    let ncols = 32;
    let n = nrows * ncols;

    let mut resistance = vec![1.0; n];
    for r in 0..nrows {
        for c in 0..ncols {
            let idx = r * ncols + c;
            if c % 8 == 0 {
                resistance[idx] = 0.001;
            }
            if r % 8 == 4 {
                resistance[idx] = 1e6;
            }
        }
    }

    let mg = build_mg(&resistance, nrows, ncols, 6);

    for level in 0..num_levels(&mg) {
        let diag_inv = level_operator(&mg, level).diag_inv();
        let mut min_d = f64::MAX;
        let mut max_d = 0.0;
        for &d in &diag_inv {
            if d < min_d {
                min_d = d;
            }
            if d > max_d {
                max_d = d;
            }
        }
        eprintln!(
            "Level {}: diag_inv [{:.4e}, {:.4e}] ratio {:.2e}",
            level,
            min_d,
            max_d,
            max_d / min_d.max(1e-30)
        );
    }

    let b_raw: Vec<f64> = (0..n)
        .map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5)
        .collect();
    let mean: f64 = b_raw.iter().sum::<f64>() / n as f64;
    let b: Vec<f64> = b_raw.iter().map(|v| v - mean).collect();

    let mut x = vec![0.0; n];
    let mut r = b.clone();
    let b_norm = b.iter().map(|v| v * v).sum::<f64>().sqrt();

    let mut residuals = Vec::new();

    for _iter in 0..20 {
        let r_norm = r.iter().map(|v| v * v).sum::<f64>().sqrt();
        residuals.push(r_norm);

        let mut z = vec![0.0; n];
        mg.apply(&r, &mut z);

        let mut az = vec![0.0; n];
        pcg::mat_vec_mul_into(fine_csmat(&mg), &z, &mut az);
        let z_az: f64 = z.iter().zip(az.iter()).map(|(a, b)| a * b).sum();
        let rz: f64 = r.iter().zip(z.iter()).map(|(a, b)| a * b).sum();
        eprintln!(
            "  iter {}: ||r||={:.4e} r·z={:.4e} z·Az={:.4e}",
            _iter, r_norm, rz, z_az
        );

        if z_az.abs() < 1e-30 {
            break;
        }

        let alpha = rz / z_az;

        for i in 0..n {
            x[i] += alpha * z[i];
            r[i] -= alpha * az[i];
        }

        if r_norm / b_norm < 1e-6 {
            break;
        }
    }

    let r0 = residuals[0];
    let r_last = residuals[residuals.len() - 1];
    eprintln!(
        "Variable coeff {}x{}: {} iters, ||r|| went from {:.6e} to {:.6e} (ratio {:.4e})",
        nrows,
        ncols,
        residuals.len(),
        r0,
        r_last,
        r_last / r0
    );
    assert!(
        r_last < r0,
        "MG-preconditioned CG diverged on variable coefficient grid: ||r|| went from {:.6e} to {:.6e}",
        r0, r_last
    );
}

#[test]
fn test_smoother_only_preconditioner() {
    let nrows = 32;
    let ncols = 32;
    let n = nrows * ncols;

    let mut resistance = vec![1.0; n];
    for r in 0..nrows {
        for c in 0..ncols {
            let idx = r * ncols + c;
            if c % 8 == 0 {
                resistance[idx] = 0.001;
            }
            if r % 8 == 4 {
                resistance[idx] = 1e6;
            }
        }
    }

    let mg = build_mg(&resistance, nrows, ncols, 6);

    let b_raw: Vec<f64> = (0..n)
        .map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5)
        .collect();
    let mean: f64 = b_raw.iter().sum::<f64>() / n as f64;
    let b: Vec<f64> = b_raw.iter().map(|v| v - mean).collect();

    let mut x = vec![0.0; n];
    let mut r = b.clone();
    let b_norm = b.iter().map(|v| v * v).sum::<f64>().sqrt();

    let mut residuals = Vec::new();

    for _iter in 0..50 {
        let r_norm = r.iter().map(|v| v * v).sum::<f64>().sqrt();
        residuals.push(r_norm);

        let mut z = vec![0.0; n];
        mg.apply(&r, &mut z);

        let mut az = vec![0.0; n];
        pcg::mat_vec_mul_into(fine_csmat(&mg), &z, &mut az);
        let z_az: f64 = z.iter().zip(az.iter()).map(|(a, b)| a * b).sum();
        let rz: f64 = r.iter().zip(z.iter()).map(|(a, b)| a * b).sum();

        if z_az.abs() < 1e-30 {
            break;
        }

        let alpha = rz / z_az;

        for i in 0..n {
            x[i] += alpha * z[i];
            r[i] -= alpha * az[i];
        }

        if r_norm / b_norm < 1e-6 {
            break;
        }
    }

    let r0 = residuals[0];
    let r_last = residuals[residuals.len() - 1];
    eprintln!(
        "Smoother-only {}x{}: {} iters, ||r|| went from {:.6e} to {:.6e} (ratio {:.4e})",
        nrows,
        ncols,
        residuals.len(),
        r0,
        r_last,
        r_last / r0
    );
}
