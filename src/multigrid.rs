use sprs::CsMat;
use crate::pcg::{Preconditioner, mat_vec_mul_slice};
use crate::cholesky;
use std::cell::RefCell;

/// Resistance value used to fill nodata cells. 1e9 Ω makes the edge
/// conductance ~1e-9 — effectively an insulator for the physics but
/// keeps every pixel as a node so the grid is fully rectangular.
const FILL_RESISTANCE: f64 = 1e9;

/// One level of the multigrid hierarchy.
struct MgLevel {
    laplacian: CsMat<f64>,
    nrows: usize,
    ncols: usize,
    cholesky_l: Option<Vec<f64>>,
    /// Prolongation triplets (rows, cols, vals) from this level to the finer level above.
    prolongation: Option<(Vec<usize>, Vec<usize>, Vec<f64>)>,
}

struct LevelWorkspace {
    z: Vec<f64>,
    r: Vec<f64>,
    rhs: Vec<f64>,
}

/// Multigrid preconditioner: applies one V-cycle as `M⁻¹·r`.
pub struct MgPreconditioner {
    levels: Vec<MgLevel>,
    nu: usize,
    omega: f64,
    workspaces: RefCell<Vec<LevelWorkspace>>, 
}

// ---------------------------------------------------------------------------
// Prolongation matrix for Galerkin coarsening
// ---------------------------------------------------------------------------

/// Build a sparse prolongation matrix P: coarse → fine.
/// Uses bilinear interpolation: each coarse cell contributes to a 4×4
/// block of fine cells with weights derived from the tensor product of
/// linear interpolation in each dimension.
///
/// Row weights: [1/4, 3/4, 3/4, 1/4]  (centered at the coarse cell)
/// Col weights: [1/4, 3/4, 3/4, 1/4]
/// The 2-D weight is the product, giving 9/16 on the four center fine
/// cells, 3/16 on the eight edge neighbours, and 1/16 on the four corners.
///
/// P has dimensions (fine_nrows * fine_ncols) x (coarse_nrows * coarse_ncols).
/// Returns (row_indices, col_indices, values) triplets.
fn build_prolongation_triplets(
    fine_nrows: usize,
    fine_ncols: usize,
    coarse_nrows: usize,
    coarse_ncols: usize,
) -> (Vec<usize>, Vec<usize>, Vec<f64>) {
    let fine_n = fine_nrows * fine_ncols;
    let mut rows = Vec::with_capacity(fine_n * 4);
    let mut cols = Vec::with_capacity(fine_n * 4);
    let mut vals = Vec::with_capacity(fine_n * 4);

    let rw: [f64; 4] = [0.25, 0.75, 0.75, 0.25];
    let cw: [f64; 4] = [0.25, 0.75, 0.75, 0.25];
    let offsets: [isize; 4] = [-1, 0, 1, 2];

    for cr in 0..coarse_nrows {
        for cc in 0..coarse_ncols {
            let coarse_idx = cr * coarse_ncols + cc;

            for (ri, &ro) in offsets.iter().enumerate() {
                let fr = 2 * cr as isize + ro;
                if fr < 0 || fr >= fine_nrows as isize { continue; }
                let fr = fr as usize;

                for (ci, &co) in offsets.iter().enumerate() {
                    let fc = 2 * cc as isize + co;
                    if fc < 0 || fc >= fine_ncols as isize { continue; }
                    let fc = fc as usize;

                    rows.push(fr * fine_ncols + fc);
                    cols.push(coarse_idx);
                    vals.push(rw[ri] * cw[ci]);
                }
            }
        }
    }

    (rows, cols, vals)
}

/// Apply prolongation: fine += P * coarse
fn prolongate_sparse(rows: &[usize], cols: &[usize], vals: &[f64], coarse: &[f64], fine: &mut [f64]) {
    for k in 0..rows.len() {
        fine[rows[k]] += vals[k] * coarse[cols[k]];
    }
}

/// Apply restriction: coarse = P^T * fine
fn restrict_sparse(rows: &[usize], cols: &[usize], vals: &[f64], fine: &[f64], coarse: &mut [f64]) {
    coarse.fill(0.0);
    for k in 0..rows.len() {
        coarse[cols[k]] += vals[k] * fine[rows[k]];
    }
}

// ---------------------------------------------------------------------------
// Matrix-dependent (Alcouffe) prolongation
// ---------------------------------------------------------------------------

fn get_conductance(lap: &CsMat<f64>, r1: usize, c1: usize, r2: usize, c2: usize, ncols: usize) -> f64 {
    let idx1 = r1 * ncols + c1;
    let idx2 = r2 * ncols + c2;
    if idx1 >= lap.rows() || idx2 >= lap.rows() {
        return 0.0;
    }
    if let Some(rv) = lap.outer_view(idx1) {
        for (col, &val) in rv.iter() {
            if col == idx2 {
                return -val;
            }
        }
    }
    0.0
}

fn push_triplet(rows: &mut Vec<usize>, cols: &mut Vec<usize>, vals: &mut Vec<f64>,
                fine: usize, coarse: usize, w: f64) {
    if w > 1e-20 {
        rows.push(fine);
        cols.push(coarse);
        vals.push(w);
    }
}

fn build_alcouffe_prolongation_triplets(
    lap: &CsMat<f64>,
    fine_nrows: usize,
    fine_ncols: usize,
    coarse_nrows: usize,
    coarse_ncols: usize,
) -> (Vec<usize>, Vec<usize>, Vec<f64>) {
    let fine_n = fine_nrows * fine_ncols;
    let mut rows = Vec::with_capacity(fine_n * 3);
    let mut cols = Vec::with_capacity(fine_n * 3);
    let mut vals = Vec::with_capacity(fine_n * 3);

    for fr in 0..fine_nrows {
        for fc in 0..fine_ncols {
            let fine_idx = fr * fine_ncols + fc;

            let d_above = if fr > 0 {
                get_conductance(lap, fr - 1, fc, fr, fc, fine_ncols)
            } else { 0.0 };
            let d_below = if fr + 1 < fine_nrows {
                get_conductance(lap, fr, fc, fr + 1, fc, fine_ncols)
            } else { 0.0 };
            let v_sum = d_above + d_below;

            let d_left = if fc > 0 {
                get_conductance(lap, fr, fc - 1, fr, fc, fine_ncols)
            } else { 0.0 };
            let d_right = if fc + 1 < fine_ncols {
                get_conductance(lap, fr, fc, fr, fc + 1, fine_ncols)
            } else { 0.0 };
            let h_sum = d_left + d_right;

            let w_v0 = if v_sum < 1e-30 { 0.5 } else { d_above / v_sum };
            let w_v1 = if v_sum < 1e-30 { 0.5 } else { d_below / v_sum };
            let w_h0 = if h_sum < 1e-30 { 0.5 } else { d_left / h_sum };
            let w_h1 = if h_sum < 1e-30 { 0.5 } else { d_right / h_sum };

            let cr0 = if fr > 0 { (fr - 1) / 2 } else { 0 };
            let cr1 = if fr + 1 < fine_nrows { (fr + 1) / 2 } else { coarse_nrows };
            let cc0 = if fc > 0 { (fc - 1) / 2 } else { 0 };
            let cc1 = if fc + 1 < fine_ncols { (fc + 1) / 2 } else { coarse_ncols };

            let ok_cr0 = cr0 < coarse_nrows && d_above > 1e-30;
            let ok_cr1 = cr1 < coarse_nrows && d_below > 1e-30;
            let ok_cc0 = cc0 < coarse_ncols && d_left > 1e-30;
            let ok_cc1 = cc1 < coarse_ncols && d_right > 1e-30;

            let full_vertical = ok_cr0 && ok_cr1;
            let full_horizontal = ok_cc0 && ok_cc1;

            if full_vertical && full_horizontal {
                push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr0 * coarse_ncols + cc0, w_v0 * w_h0);
                push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr0 * coarse_ncols + cc1, w_v0 * w_h1);
                push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr1 * coarse_ncols + cc0, w_v1 * w_h0);
                push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr1 * coarse_ncols + cc1, w_v1 * w_h1);
            } else if full_vertical {
                if ok_cc0 {
                    push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr0 * coarse_ncols + cc0, w_v0 * 1.0);
                    push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr1 * coarse_ncols + cc0, w_v1 * 1.0);
                } else if ok_cc1 {
                    push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr0 * coarse_ncols + cc1, w_v0 * 1.0);
                    push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr1 * coarse_ncols + cc1, w_v1 * 1.0);
                } else {
                    push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr0 * coarse_ncols + cc0, 1.0);
                }
            } else if full_horizontal {
                if ok_cr0 {
                    push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr0 * coarse_ncols + cc0, 1.0 * w_h0);
                    push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr0 * coarse_ncols + cc1, 1.0 * w_h1);
                } else if ok_cr1 {
                    push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr1 * coarse_ncols + cc0, 1.0 * w_h0);
                    push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr1 * coarse_ncols + cc1, 1.0 * w_h1);
                } else {
                    push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr0 * coarse_ncols + cc0, 1.0);
                }
            } else {
                if ok_cr0 && ok_cc0 {
                    push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr0 * coarse_ncols + cc0, 1.0);
                } else if ok_cr0 && ok_cc1 {
                    push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr0 * coarse_ncols + cc1, 1.0);
                } else if ok_cr1 && ok_cc0 {
                    push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr1 * coarse_ncols + cc0, 1.0);
                } else if ok_cr1 && ok_cc1 {
                    push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr1 * coarse_ncols + cc1, 1.0);
                } else {
                    push_triplet(&mut rows, &mut cols, &mut vals, fine_idx, cr0 * coarse_ncols + cc0, 1.0);
                }
            }
        }
    }

    (rows, cols, vals)
}

/// Replace nodata / non-positive / non-finite resistance with
/// `FILL_RESISTANCE` so every cell becomes a graph node.
pub fn fill_nodata(data: &[f64], nodata: f64) -> Vec<f64> {
    data.iter()
        .map(|&v| {
            if v == nodata || !v.is_finite() || v <= 0.0 {
                FILL_RESISTANCE
            } else {
                v
            }
        })
        .collect()
}

/// Which prolongation operator to use during Galerkin coarsening.
#[derive(Copy, Clone, PartialEq)]
enum ProlongKind {
    /// Fixed bilinear interpolation weights.
    Bilinear,
    /// Alcouffe operator-induced weights derived from the fine Laplacian.
    Alcouffe,
}

impl MgPreconditioner {
    /// Build a multigrid hierarchy directly from a fine-grid system matrix.
    ///
    /// The matrix is used as-is for level 0 and Galerkin-coarsened
    /// (`A_coarse = Pᵀ · A · P`) for deeper levels
    pub fn build_from_laplacian(
        a: &CsMat<f64>,
        nrows: usize,
        ncols: usize,
        max_levels: usize,
    ) -> Self {
        Self::from_laplacian_impl(a, nrows, ncols, max_levels, ProlongKind::Bilinear, None)
    }

    /// Alcouffe variant of `build_from_laplacian`. `weights` optionally
    /// supplies a separate matrix for deriving the first-level prolongation
    /// weights (e.g. the un-pinned Laplacian when `a` has Dirichlet identity
    /// rows); defaults to `a`.
    pub fn build_alcouffe_from_laplacian(
        a: &CsMat<f64>,
        weights: Option<&CsMat<f64>>,
        nrows: usize,
        ncols: usize,
        max_levels: usize,
    ) -> Self {
        Self::from_laplacian_impl(a, nrows, ncols, max_levels, ProlongKind::Alcouffe, weights)
    }

    fn from_laplacian_impl(
        a: &CsMat<f64>,
        nrows: usize,
        ncols: usize,
        max_levels: usize,
        prolong: ProlongKind,
        weights_lap: Option<&CsMat<f64>>,
    ) -> Self {
        debug_assert_eq!(
            a.rows(), nrows * ncols,
            "MG hierarchy expects all cells as nodes, got {} vs {}",
            a.rows(), nrows * ncols
        );

        let mut levels = Vec::new();
        let (mut nr, mut nc) = (nrows, ncols);

        levels.push(MgLevel {
            laplacian: a.clone(),
            nrows: nr,
            ncols: nc,
            cholesky_l: None,
            prolongation: None,
        });

        // Build deeper levels using Galerkin coarsening
        while levels.len() < max_levels {
            let fine_nr = nr;
            let fine_nc = nc;
            let next_nr = fine_nr / 2;
            let next_nc = fine_nc / 2;
            if next_nr < 4 || next_nc < 4 {
                break;
            }

            let fine_lap = &levels.last().unwrap().laplacian;
            let (p_rows, p_cols, p_vals) = match prolong {
                ProlongKind::Bilinear =>
                    build_prolongation_triplets(fine_nr, fine_nc, next_nr, next_nc),
                ProlongKind::Alcouffe => {
                    // For the first prolongation (level 0 -> 1), use the
                    // weights matrix if supplied (avoids Dirichlet-identity
                    // rows corrupting Alcouffe weights).
                    let lap_for_prolong = if levels.len() == 1 {
                        weights_lap.unwrap_or(fine_lap)
                    } else {
                        fine_lap
                    };
                    build_alcouffe_prolongation_triplets(lap_for_prolong, fine_nr, fine_nc, next_nr, next_nc)
                }
            };
            let fine_n = fine_nr * fine_nc;
            let coarse_n = next_nr * next_nc;

            // Build prolongation as a sparse matrix for Galerkin
            let p_tri = sprs::TriMat::from_triplets((fine_n, coarse_n), p_rows.clone(), p_cols.clone(), p_vals.clone());
            let p = p_tri.to_csr();

            // Galerkin: L_coarse = P^T * L_fine * P
            let pt = p.transpose_view().to_csr();
            let pt_lap = sprs::smmp::mul_csr_csr::<f64, f64, f64, usize, usize>(pt.view(), fine_lap.view());
            let mut laplacian = sprs::smmp::mul_csr_csr::<f64, f64, f64, usize, usize>(pt_lap.view(), p.view());
            crate::laplacian::regularize_laplacian(&mut laplacian);

            levels.push(MgLevel {
                laplacian,
                nrows: next_nr,
                ncols: next_nc,
                cholesky_l: None,
                prolongation: Some((p_rows, p_cols, p_vals)),
            });

            nr = next_nr;
            nc = next_nc;
        }

        // Compute Cholesky factorization on the coarsest level
        let coarsest_idx = levels.len() - 1;
        let cnodes = {
            let c = &levels[coarsest_idx];
            c.nrows * c.ncols
        };
        let dense = cholesky::sparse_to_dense(&levels[coarsest_idx].laplacian, cnodes);
        match cholesky::cholesky_decompose(&dense, cnodes) {
            Some(l) => {
                levels[coarsest_idx].cholesky_l = Some(l);
            }
            None => {
                // Cholesky failed (non-SPD coarsest level); the V-cycle falls
                // back to a few Jacobi-preconditioned CG iterations.
            }
        }

        // Pre-allocate scratch workspaces
        let workspaces = levels.iter().map(|lvl| {
            let n = lvl.nrows * lvl.ncols;
            LevelWorkspace {
                z: vec![0.0; n],
                r: vec![0.0; n],
                rhs: vec![0.0; n],
            }
        }).collect();

        Self {
            levels,
            nu: 2,
            omega: 0.67,
            workspaces: RefCell::new(workspaces),
        }
    }

    // -----------------------------------------------------------------------
    // V-cycle
    // -----------------------------------------------------------------------

    fn v_cycle(&self, workspaces: &RefCell<Vec<LevelWorkspace>>, b: &[f64], level: usize) {
        let lvl = &self.levels[level];
        let n = lvl.nrows * lvl.ncols;

        if level == self.levels.len() - 1 {
            if let Some(ref l) = lvl.cholesky_l {
                let x = crate::cholesky::cholesky_solve(l, b, n);
                workspaces.borrow_mut()[level].z.copy_from_slice(&x);
            } else {
                let mut ws = workspaces.borrow_mut();
                let res = crate::pcg::cg_solve(&lvl.laplacian, b, 50, 1e-3, Some(&ws[level].z));
                ws[level].z.copy_from_slice(&res.x);
            }
            return;
        }

        {
            let mut ws = workspaces.borrow_mut();
            ws[level].z.fill(0.0);
            if b.as_ptr() != ws[level].rhs.as_ptr() {
                ws[level].rhs.copy_from_slice(b);
            }
        }

        for _ in 0..self.nu {
            let mut ws = workspaces.borrow_mut();
            unsafe {
                let ptr = ws.as_mut_ptr().add(level);
                let z = std::slice::from_raw_parts_mut((*ptr).z.as_mut_ptr(), n);
                let rhs = std::slice::from_raw_parts((*ptr).rhs.as_ptr(), n);
                symmetric_gauss_seidel_smooth(&lvl.laplacian, z, rhs, self.omega);
            }
            drop(ws);
        }

        {
            let mut ws = workspaces.borrow_mut();
            unsafe {
                let ptr = ws.as_mut_ptr().add(level);
                let z = std::slice::from_raw_parts((*ptr).z.as_ptr(), n);
                let r = std::slice::from_raw_parts_mut((*ptr).r.as_mut_ptr(), n);
                let rhs = std::slice::from_raw_parts((*ptr).rhs.as_ptr(), n);
                mat_vec_mul_slice(&lvl.laplacian, z, r);
                for i in 0..n {
                    r[i] = rhs[i] - r[i];
                }
            }
        }

        let next_b_ptr: *const f64;
        let next_b_len: usize;
        {
            let mut ws = workspaces.borrow_mut();
            let ws_slice = ws.as_mut_slice();
            let (left, right) = ws_slice.split_at_mut(level + 1);
            let fine_ws = &left[level];
            let coarse_ws = &mut right[0];
            let (p_rows, p_cols, p_vals) = self.levels[level + 1].prolongation.as_ref().unwrap();
            coarse_ws.rhs.fill(0.0);
            restrict_sparse(p_rows, p_cols, p_vals, &fine_ws.r, &mut coarse_ws.rhs);
            next_b_ptr = coarse_ws.rhs.as_ptr();
            next_b_len = coarse_ws.rhs.len();
        }

        let next_b: &[f64] = unsafe { std::slice::from_raw_parts(next_b_ptr, next_b_len) };
        self.v_cycle(workspaces, next_b, level + 1);

        let next_lvl = &self.levels[level + 1];
        {
            let mut ws = workspaces.borrow_mut();
            let ws_slice = ws.as_mut_slice();
            let (left, right) = ws_slice.split_at_mut(level + 1);
            let fine_ws = &mut left[level];
            let coarse_ws = &right[0];
            let (p_rows, p_cols, p_vals) = next_lvl.prolongation.as_ref().unwrap();
            fine_ws.r.fill(0.0);
            prolongate_sparse(p_rows, p_cols, p_vals, &coarse_ws.z, &mut fine_ws.r);
            for i in 0..n {
                fine_ws.z[i] += fine_ws.r[i];
            }
        }

        for _ in 0..self.nu {
            let mut ws = workspaces.borrow_mut();
            unsafe {
                let ptr = ws.as_mut_ptr().add(level);
                let z = std::slice::from_raw_parts_mut((*ptr).z.as_mut_ptr(), n);
                let rhs = std::slice::from_raw_parts((*ptr).rhs.as_ptr(), n);
                symmetric_gauss_seidel_smooth(&lvl.laplacian, z, rhs, self.omega);
            }
            drop(ws);
        }
    }
}

impl Preconditioner for MgPreconditioner {
    fn apply(&self, r: &[f64], z: &mut Vec<f64>) {
        self.v_cycle(&self.workspaces, r, 0);

        let ws = self.workspaces.borrow_mut();
        z.resize(r.len(), 0.0);
        z.copy_from_slice(&ws[0].z);
    }
}

fn symmetric_gauss_seidel_smooth(
    laplacian: &CsMat<f64>,
    x: &mut [f64],
    b: &[f64],
    omega: f64,
) {
    let n = b.len();
    // Forward sweep
    for row in 0..n {
        if let Some(rv) = laplacian.outer_view(row) {
            let mut diag = 0.0;
            let mut off_diag_sum = 0.0;
            for (col, &val) in rv.iter() {
                if col == row {
                    diag = val;
                } else {
                    off_diag_sum += val * x[col];
                }
            }
            if diag.abs() > 1e-15 {
                let new_x = (b[row] - off_diag_sum) / diag;
                x[row] = (1.0 - omega) * x[row] + omega * new_x;
            }
        }
    }
    // Backward sweep
    for row in (0..n).rev() {
        if let Some(rv) = laplacian.outer_view(row) {
            let mut diag = 0.0;
            let mut off_diag_sum = 0.0;
            for (col, &val) in rv.iter() {
                if col == row {
                    diag = val;
                } else {
                    off_diag_sum += val * x[col];
                }
            }
            if diag.abs() > 1e-15 {
                let new_x = (b[row] - off_diag_sum) / diag;
                x[row] = (1.0 - omega) * x[row] + omega * new_x;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pcg;

    // Helper to generate safe mock resistance data for a uniform grid
    fn generate_mock_resistance(nrows: usize, ncols: usize) -> Vec<f64> {
        vec![1.0; nrows * ncols]
    }

    // Build a bilinear MG hierarchy from a resistance raster (nodata -1.0).
    fn build_mg(resistance: &[f64], nrows: usize, ncols: usize, max_levels: usize) -> MgPreconditioner {
        let filled = fill_nodata(resistance, -1.0);
        let (_cell_to_node, _num_nodes, _edges, laplacian, _components) =
            crate::build_circuit_model(&filled, nrows, ncols, -1.0);
        MgPreconditioner::build_from_laplacian(&laplacian, nrows, ncols, max_levels)
    }

    // Build an Alcouffe MG hierarchy from a resistance raster (nodata -1.0).
    fn build_mg_alcouffe(resistance: &[f64], nrows: usize, ncols: usize, max_levels: usize) -> MgPreconditioner {
        let filled = fill_nodata(resistance, -1.0);
        let (_cell_to_node, _num_nodes, _edges, laplacian, _components) =
            crate::build_circuit_model(&filled, nrows, ncols, -1.0);
        MgPreconditioner::build_alcouffe_from_laplacian(&laplacian, None, nrows, ncols, max_levels)
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
        pcg::mat_vec_mul_into(&mg.levels[0].laplacian, &x, &mut ax);
        pcg::mat_vec_mul_into(&mg.levels[0].laplacian, &y, &mut ay);
        let dot_x_ay: f64 = x.iter().zip(ay.iter()).map(|(a, b)| a * b).sum();
        let dot_y_ax: f64 = y.iter().zip(ax.iter()).map(|(a, b)| a * b).sum();
        assert!((dot_x_ay - dot_y_ax).abs() < 1e-7, "Fine matrix A is asymmetric!");

        mg.apply(&x, &mut mx);
        mg.apply(&y, &mut my);
        
        // 3. Compute dot products: x · M(y) vs y · M(x)
        let dot_x_my: f64 = x.iter().zip(my.iter()).map(|(a, b)| a * b).sum();
        let dot_y_mx: f64 = y.iter().zip(mx.iter()).map(|(a, b)| a * b).sum();
        
        // Operators must be symmetric up to floating-point drift
        assert!(
            (dot_x_my - dot_y_mx).abs() < 1e-6, 
            "MG Preconditioner is asymmetric! x*M(y) = {}, y*M(x) = {}", dot_x_my, dot_y_mx
        );
    }

    #[test]
    fn test_mg_preconditioned_cg_converges() {
        let nrows = 64;
        let ncols = 64;
        let n = nrows * ncols;

        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = build_mg(&resistance, nrows, ncols, 7);

        let b_raw: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5).collect();
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
            pcg::mat_vec_mul_into(&mg.levels[0].laplacian, &z, &mut az);
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
        eprintln!("Large uniform {}x{}: {} iters, ||r|| went from {:.6e} to {:.6e} (ratio {:.4e})",
            nrows, ncols, residuals.len(), r0, r_last, r_last / r0);
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

        for (level_idx, lvl) in mg.levels.iter().enumerate() {
            let n = lvl.nrows * lvl.ncols;
            for row in 0..n {
                if let Some(rv) = lvl.laplacian.outer_view(row) {
                    for (col, &val) in rv.iter() {
                        if col == row {
                            assert!(
                                val > 0.0,
                                "Level {}: diagonal[{}] = {:.6e} (must be positive)",
                                level_idx, row, val
                            );
                            let diag_inv = if val.abs() > 1e-15 { 1.0 / val.abs() } else { 0.0 };
                            assert!(
                                diag_inv > 0.0,
                                "Level {}: diag_inv[{}] = {:.6e} (must be positive)",
                                level_idx, row, diag_inv
                            );
                        }
                    }
                }
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

        let z: Vec<f64> = (0..n).map(|i| (i as f64 * 0.7 + 1.3).sin() * 0.5 + 0.3).collect();
        let mut mz = vec![0.0; n];
        mg.apply(&z, &mut mz);

        let z_mz: f64 = z.iter().zip(mz.iter()).map(|(a, b)| a * b).sum();
        assert!(
            z_mz > 0.0,
            "Preconditioner is not positive definite: zᵀ·M⁻¹·z = {:.6e}",
            z_mz
        );

        let z2: Vec<f64> = (0..n).map(|i| (i as f64 * 1.1 + 2.7).cos() * 0.8 - 0.2).collect();
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

        let b_raw: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5).collect();
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

        let coarsest = mg.levels.len() - 1;
        let lvl = &mg.levels[coarsest];
        let n = lvl.nrows * lvl.ncols;

        assert!(
            lvl.cholesky_l.is_some(),
            "Coarsest level should have Cholesky factorization"
        );

        let b: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.5 + 0.3).sin() + 1.0) * 0.5).collect();
        let l = lvl.cholesky_l.as_ref().unwrap();
        let z = crate::cholesky::cholesky_solve(l, &b, n);

        let mut az = vec![0.0; n];
        pcg::mat_vec_mul_into(&lvl.laplacian, &z, &mut az);

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

        for (level_idx, lvl) in mg.levels.iter().enumerate() {
            let n = lvl.nrows * lvl.ncols;
            let b = vec![0.0; n];
            let mut x: Vec<f64> = (0..n).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();

            let initial_norm = x.iter().map(|v| v * v).sum::<f64>().sqrt();
            for _ in 0..3 {
                symmetric_gauss_seidel_smooth(&lvl.laplacian, &mut x, &b, mg.omega);
            }
            let final_norm = x.iter().map(|v| v * v).sum::<f64>().sqrt();

            assert!(
                final_norm < initial_norm,
                "Level {}: smoother did not reduce error norm (initial={:.6e}, final={:.6e})",
                level_idx, initial_norm, final_norm
            );
        }
    }

    // -----------------------------------------------------------------------
    // Diagnostic tests for MG preconditioner on subgraph vs full-grid Laplacian
    // -----------------------------------------------------------------------

    #[test]
    fn test_mg_preconditioner_on_subgraph_mismatch() {
        let nrows = 8;
        let ncols = 8;
        let n = nrows * ncols;

        let resistance = generate_mock_resistance(nrows, ncols);
        let (_cell_to_node, num_nodes, _edges, full_lap, components) =
            crate::build_circuit_model(&resistance, nrows, ncols, -1.0);
        assert_eq!(num_nodes, n);

        let mg = build_mg(&resistance, nrows, ncols, 4);

        let comp = &components[0];
        let (a_local, _node_to_local) = crate::components::build_subgraph_laplacian(&full_lap, comp);
        let comp_size = comp.len();

        let b_raw: Vec<f64> = (0..comp_size).map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5).collect();
        let mean: f64 = b_raw.iter().sum::<f64>() / comp_size as f64;
        let b: Vec<f64> = b_raw.iter().map(|v| v - mean).collect();

        let mut z = vec![0.0; comp_size];
        mg.apply(&b, &mut z);

        let mut az = vec![0.0; comp_size];
        pcg::mat_vec_mul_into(&a_local, &z, &mut az);
        let z_az: f64 = z.iter().zip(az.iter()).map(|(a, b)| a * b).sum();
        let rz: f64 = b.iter().zip(z.iter()).map(|(a, b)| a * b).sum();

        let b_norm = b.iter().map(|v| v * v).sum::<f64>().sqrt();
        eprintln!("Subgraph test: comp_size={}, full_grid={}", comp_size, n);
        eprintln!("  ||b|| = {:.6e}, r·z = {:.6e}, z·Az = {:.6e}", b_norm, rz, z_az);
        eprintln!("  alpha = {:.6e}", rz / z_az.max(1e-30));

        assert!(
            z_az > 0.0,
            "MG preconditioner on subgraph: z·Az = {:.6e} (must be > 0 for CG descent)",
            z_az
        );
        assert!(
            rz > 0.0,
            "MG preconditioner on subgraph: r·z = {:.6e} (must be > 0 for CG descent)",
            rz
        );
    }

    #[test]
    fn test_mg_preconditioner_on_full_grid() {
        let nrows = 8;
        let ncols = 8;
        let n = nrows * ncols;

        let resistance = generate_mock_resistance(nrows, ncols);
        let (_cell_to_node, num_nodes, _edges, _full_lap, _components) =
            crate::build_circuit_model(&resistance, nrows, ncols, -1.0);
        assert_eq!(num_nodes, n);

        let mg = build_mg(&resistance, nrows, ncols, 4);

        let b_raw: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5).collect();
        let mean: f64 = b_raw.iter().sum::<f64>() / n as f64;
        let b: Vec<f64> = b_raw.iter().map(|v| v - mean).collect();

        let mut z = vec![0.0; n];
        mg.apply(&b, &mut z);

        let mut az = vec![0.0; n];
        pcg::mat_vec_mul_into(&mg.levels[0].laplacian, &z, &mut az);
        let z_az: f64 = z.iter().zip(az.iter()).map(|(a, b)| a * b).sum();
        let rz: f64 = b.iter().zip(z.iter()).map(|(a, b)| a * b).sum();

        let b_norm = b.iter().map(|v| v * v).sum::<f64>().sqrt();
        eprintln!("Full-grid test: n={}", n);
        eprintln!("  ||b|| = {:.6e}, r·z = {:.6e}, z·Az = {:.6e}", b_norm, rz, z_az);

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
    fn test_single_component_equals_full_grid() {
        let nrows = 4;
        let ncols = 4;
        let n = nrows * ncols;

        let resistance = generate_mock_resistance(nrows, ncols);
        let (_cell_to_node, _num_nodes, _edges, full_lap, components) =
            crate::build_circuit_model(&resistance, nrows, ncols, -1.0);

        assert_eq!(components.len(), 1, "Uniform grid should have exactly 1 component");
        assert_eq!(components[0].len(), n, "Component should include all nodes");

        let (a_local, _node_to_local) = crate::components::build_subgraph_laplacian(&full_lap, &components[0]);

        // Build inverse map: local index -> global index
        let comp = &components[0];
        let mut max_diff = 0.0;
        for local_u in 0..n {
            let global_u = comp[local_u];
            if let Some(rv_local) = a_local.outer_view(local_u) {
                if let Some(rv_full) = full_lap.outer_view(global_u) {
                    // Compare each entry: local (local_v, val) vs full (global_v, val)
                    for (local_v, &val) in rv_local.iter() {
                        let global_v = comp[local_v];
                        // Find the matching entry in the full Laplacian
                        let mut found = false;
                        for (full_col, &full_val) in rv_full.iter() {
                            if full_col == global_v {
                                let diff = (val - full_val).abs();
                                if diff > max_diff {
                                    max_diff = diff;
                                }
                                found = true;
                                break;
                            }
                        }
                        assert!(found, "Local entry ({},{}) -> global ({},{}) not found in full Laplacian",
                            local_u, local_v, global_u, global_v);
                    }
                }
            }
        }
        assert!(
            max_diff < 1e-10,
            "Subgraph Laplacian differs from full-grid Laplacian for single component: max_diff = {:.6e}",
            max_diff
        );
    }

    #[test]
    fn test_diag_inv_range_across_levels() {
        let nrows = 32;
        let ncols = 32;
        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = build_mg(&resistance, nrows, ncols, 6);

        for (level_idx, lvl) in mg.levels.iter().enumerate() {
            let diag_inv = crate::laplacian::extract_diag_inv(&lvl.laplacian);
            let n = lvl.nrows * lvl.ncols;
            let mut min_inv = f64::MAX;
            let mut max_inv = 0.0;
            for i in 0..n {
                if diag_inv[i] < min_inv {
                    min_inv = diag_inv[i];
                }
                if diag_inv[i] > max_inv {
                    max_inv = diag_inv[i];
                }
            }
            eprintln!("Level {}: diag_inv range = [{:.6e}, {:.6e}] (ratio = {:.2e})",
                level_idx, min_inv, max_inv, max_inv / min_inv.max(1e-30));

            assert!(
                min_inv > 1e-15,
                "Level {}: diag_inv too small: min = {:.6e}",
                level_idx, min_inv
            );
            assert!(
                max_inv < 1e10,
                "Level {}: diag_inv too large: max = {:.6e}",
                level_idx, max_inv
            );
        }
    }

    #[test]
    fn test_coarse_cholesky_accuracy() {
        let nrows = 32;
        let ncols = 32;
        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = build_mg(&resistance, nrows, ncols, 6);

        let coarsest = mg.levels.len() - 1;
        let lvl = &mg.levels[coarsest];
        let n = lvl.nrows * lvl.ncols;

        assert!(
            lvl.cholesky_l.is_some(),
            "Coarsest level should have Cholesky factorization"
        );

        let l = lvl.cholesky_l.as_ref().unwrap();
        let b: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.5 + 0.3).sin() + 1.0) * 0.5).collect();
        let x = crate::cholesky::cholesky_solve(l, &b, n);

        let mut ax = vec![0.0; n];
        pcg::mat_vec_mul_into(&lvl.laplacian, &x, &mut ax);

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

        for (level_idx, lvl) in mg.levels.iter().enumerate() {
            let diag_inv = crate::laplacian::extract_diag_inv(&lvl.laplacian);
            let nn = lvl.nrows * lvl.ncols;
            let mut min_d = f64::MAX;
            let mut max_d = 0.0;
            for i in 0..nn {
                if diag_inv[i] < min_d { min_d = diag_inv[i]; }
                if diag_inv[i] > max_d { max_d = diag_inv[i]; }
            }
            eprintln!("Level {}: diag_inv [{:.4e}, {:.4e}] ratio {:.2e}",
                level_idx, min_d, max_d, max_d / min_d.max(1e-30));
        }

        let b_raw: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5).collect();
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
            pcg::mat_vec_mul_into(&mg.levels[0].laplacian, &z, &mut az);
            let z_az: f64 = z.iter().zip(az.iter()).map(|(a, b)| a * b).sum();
            let rz: f64 = r.iter().zip(z.iter()).map(|(a, b)| a * b).sum();
            eprintln!("  iter {}: ||r||={:.4e} r·z={:.4e} z·Az={:.4e}",
                _iter, r_norm, rz, z_az);

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
        eprintln!("Variable coeff {}x{}: {} iters, ||r|| went from {:.6e} to {:.6e} (ratio {:.4e})",
            nrows, ncols, residuals.len(), r0, r_last, r_last / r0);
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

        let b_raw: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5).collect();
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
            pcg::mat_vec_mul_into(&mg.levels[0].laplacian, &z, &mut az);
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
        eprintln!("Smoother-only {}x{}: {} iters, ||r|| went from {:.6e} to {:.6e} (ratio {:.4e})",
            nrows, ncols, residuals.len(), r0, r_last, r_last / r0);
    }

    // -------------------------------------------------------------------
    // Alcouffe matrix-dependent prolongation tests
    // -------------------------------------------------------------------

    #[test]
    fn test_alcouffe_weights_partition_uniform() {
        let nrows = 16;
        let ncols = 16;
        let n = nrows * ncols;
        let resistance = vec![1.0; n];

        let mg = build_mg_alcouffe(&resistance, nrows, ncols, 4);
        let ap = mg.levels[1].prolongation.as_ref().unwrap();

        let fine_n = nrows * ncols;
        let mut col_sum = vec![0.0; fine_n];
        for k in 0..ap.0.len() {
            assert!(ap.2[k] > 0.0 && ap.2[k] <= 1.0001,
                "Alcouffe weight out of (0,1]: {}", ap.2[k]);
            col_sum[ap.0[k]] += ap.2[k];
        }

        let border = 2;
        let mut checked = 0;
        for fr in border..(nrows - border) {
            for fc in border..(ncols - border) {
                let idx = fr * ncols + fc;
                assert!(
                    (col_sum[idx] - 1.0).abs() < 1e-12,
                    "Alcouffe column sum for interior ({},{}) is {} (expected 1.0)",
                    fr, fc, col_sum[idx]
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "No interior fine nodes found");
    }

    #[test]
    fn test_alcouffe_differs_on_variable() {
        let nrows = 16;
        let ncols = 16;
        let n = nrows * ncols;

        let uniform = vec![1.0; n];
        let mut variable = vec![1.0; n];
        for r in 0..nrows {
            for c in 0..ncols {
                let idx = r * ncols + c;
                if c % 4 == 0 {
                    variable[idx] = 1000.0;
                }
            }
        }

        let mg_uni = build_mg_alcouffe(&uniform, nrows, ncols, 4);
        let mg_var = build_mg_alcouffe(&variable, nrows, ncols, 4);

        let up = mg_uni.levels[1].prolongation.as_ref().unwrap();
        let vp = mg_var.levels[1].prolongation.as_ref().unwrap();

        assert_eq!(up.0.len(), vp.0.len(),
            "Alcouffe P sizes differ: uniform {} vs variable {}",
            up.0.len(), vp.0.len());

        let mut total_diff = 0.0;
        for k in 0..up.0.len() {
            assert_eq!(up.0[k], vp.0[k], "row mismatch at {}", k);
            assert_eq!(up.1[k], vp.1[k], "col mismatch at {}", k);
            total_diff += (up.2[k] - vp.2[k]).abs();
        }
        assert!(
            total_diff > 1e-6,
            "Alcouffe weights should differ on variable coeff grid; total diff = {:.6e}",
            total_diff
        );
    }

    #[test]
    fn test_alcouffe_preconditioned_cg_converges_variable() {
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

        let mg = build_mg_alcouffe(&resistance, nrows, ncols, 6);

        for (level_idx, lvl) in mg.levels.iter().enumerate() {
            let diag_inv = crate::laplacian::extract_diag_inv(&lvl.laplacian);
            let nn = lvl.nrows * lvl.ncols;
            let mut min_d = f64::MAX;
            let mut max_d = 0.0;
            for i in 0..nn {
                if diag_inv[i] < min_d { min_d = diag_inv[i]; }
                if diag_inv[i] > max_d { max_d = diag_inv[i]; }
            }
            eprintln!("Alcouffe level {}: diag_inv [{:.4e}, {:.4e}] ratio {:.2e}",
                level_idx, min_d, max_d, max_d / min_d.max(1e-30));
        }

        let b_raw: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5).collect();
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
            pcg::mat_vec_mul_into(&mg.levels[0].laplacian, &z, &mut az);
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
        eprintln!("Alcouffe variable coeff {}x{}: {} iters, ||r|| went from {:.6e} to {:.6e} (ratio {:.4e})",
            nrows, ncols, residuals.len(), r0, r_last, r_last / r0);
        assert!(
            r_last < r0,
            "Alcouffe MG-preconditioned CG diverged on variable coefficient grid: ||r|| went from {:.6e} to {:.6e}",
            r0, r_last
        );
    }

    #[test]
    fn test_alcouffe_symmetry() {
        let nrows = 15;
        let ncols = 15;
        let n = nrows * ncols;

        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = build_mg_alcouffe(&resistance, nrows, ncols, 3);

        let x: Vec<f64> = (0..n).map(|i| (i as f64 * 0.1).sin()).collect();
        let y: Vec<f64> = (0..n).map(|i| (i as f64 * 0.2).cos()).collect();

        let mut mx = vec![0.0; n];
        let mut my = vec![0.0; n];

        let mut ax = vec![0.0; n];
        let mut ay = vec![0.0; n];
        pcg::mat_vec_mul_into(&mg.levels[0].laplacian, &x, &mut ax);
        pcg::mat_vec_mul_into(&mg.levels[0].laplacian, &y, &mut ay);
        let dot_x_ay: f64 = x.iter().zip(ay.iter()).map(|(a, b)| a * b).sum();
        let dot_y_ax: f64 = y.iter().zip(ax.iter()).map(|(a, b)| a * b).sum();
        assert!((dot_x_ay - dot_y_ax).abs() < 1e-7, "Fine matrix A is asymmetric!");

        mg.apply(&x, &mut mx);
        mg.apply(&y, &mut my);

        let dot_x_my: f64 = x.iter().zip(my.iter()).map(|(a, b)| a * b).sum();
        let dot_y_mx: f64 = y.iter().zip(mx.iter()).map(|(a, b)| a * b).sum();

        assert!(
            (dot_x_my - dot_y_mx).abs() < 1e-6,
            "Alcouffe MG Preconditioner is asymmetric! x*M(y) = {}, y*M(x) = {}",
            dot_x_my, dot_y_mx
        );
    }
}
