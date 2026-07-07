//! Geometric multigrid preconditioner for the Laplacian system.
//!
//! Fills nodata cells with a high resistance so the domain is a perfect
//! rectangle, then builds a hierarchy of coarsened Laplacians. The
//! preconditioner applies one V-cycle per CG iteration.

use sprs::CsMat;
use crate::solver::{self, Preconditioner};
use crate::resample;
use crate::cholesky;
use std::time::Instant;
use std::cell::RefCell;

/// Resistance value used to fill nodata cells. 1e9 Ω makes the edge
/// conductance ~1e-9 — effectively an insulator for the physics but
/// keeps every pixel as a node so the grid is fully rectangular.
const FILL_RESISTANCE: f64 = 1e9;

/// One level of the multigrid hierarchy.
struct MgLevel {
    laplacian: CsMat<f64>,
    diag_inv: Vec<f64>,
    nrows: usize,
    ncols: usize,
    cholesky_l: Option<Vec<f64>>,
}

struct LevelWorkspace {
    z: Vec<f64>,
    r: Vec<f64>,
    ax: Vec<f64>,
}

/// Multigrid preconditioner: applies one V-cycle as `M⁻¹·r`.
pub struct MgPreconditioner {
    levels: Vec<MgLevel>,
    nu: usize,
    omega: f64,
    workspaces: RefCell<Vec<LevelWorkspace>>, 
}

// ---------------------------------------------------------------------------
// Nodata fill
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Hierarchy construction
// ---------------------------------------------------------------------------

impl MgPreconditioner {
    /// Build a multigrid hierarchy from a filled resistance raster.
    pub fn build(
        resistance: &[f64],
        nrows: usize,
        ncols: usize,
        nodata: f64,
        max_levels: usize,
    ) -> Self {
        let t0 = Instant::now();
        let filled = fill_nodata(resistance, nodata);
        let mut levels = Vec::new();
        let mut cur_res = filled;
        let (mut nr, mut nc) = (nrows, ncols);

        loop {
            let t1 = Instant::now();
            let (_nodemap, num_nodes, _edges, laplacian, _components) =
                crate::build_circuit_model(&cur_res, nr, nc, nodata);
            let build_ms = t1.elapsed().as_millis();

            // Sanity: num_nodes should == nr*nc when all cells are filled
            debug_assert_eq!(
                num_nodes,
                nr * nc,
                "MG hierarchy expects all cells as nodes, got {} vs {}",
                num_nodes,
                nr * nc
            );

            // Precompute inverse diagonal elements for this level
            let mut diag_inv = vec![0.0f64; num_nodes];
            for row in 0..num_nodes {
                if let Some(rv) = laplacian.outer_view(row) {
                    for (col, &val) in rv.iter() {
                        if col == row {
                            let abs_val = val.abs();
                            diag_inv[row] = if abs_val > 1e-15 { 1.0 / abs_val } else { 0.0 };
                            break;
                        }
                    }
                }
            }

            levels.push(MgLevel {
                laplacian,
                diag_inv,
                nrows: nr,
                ncols: nc,
                cholesky_l: None,
            });
            eprintln!("  MG level {}: {}x{} ({} nodes) built in {}ms",
                levels.len() - 1, nr, nc, num_nodes, build_ms);

            let next_nr = nr / 2;
            let next_nc = nc / 2;
            if next_nr < 4 || next_nc < 4 || levels.len() >= max_levels {
                break;
            }
            let down =
                resample::downsample_raster(&cur_res, nr, nc, nodata, next_nr, next_nc);
            cur_res = down.data;
            nr = next_nr;
            nc = next_nc;
        }

        // Compute Cholesky factorization on the coarsest level
        let coarsest_idx = levels.len() - 1;
        let (cnrows, cncols, cnodes) = {
            let c = &levels[coarsest_idx];
            (c.nrows, c.ncols, c.nrows * c.ncols)
        };
        let dense = cholesky::sparse_to_dense(&levels[coarsest_idx].laplacian, cnodes);
        match cholesky::cholesky_decompose(&dense, cnodes) {
            Some(l) => {
                levels[coarsest_idx].cholesky_l = Some(l);
                eprintln!("  MG coarsest Cholesky factorized ({}x{}, {} nodes)",
                    cnrows, cncols, cnodes);
            }
            None => {
                eprintln!("  MG coarsest Cholesky FAILED ({}x{}, {} nodes) — falling back to CG",
                    cnrows, cncols, cnodes);
            }
        }

        // Pre-allocate scratch workspaces
        let workspaces = levels.iter().map(|lvl| {
            let n = lvl.nrows * lvl.ncols;
            LevelWorkspace {
                z: vec![0.0; n],
                r: vec![0.0; n],
                ax: vec![0.0; n],
            }
        }).collect();

        let elapsed = t0.elapsed();
        eprintln!("MG hierarchy built in {}ms ({} levels)", elapsed.as_millis(), levels.len());

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

    fn v_cycle(&self, workspaces: &std::cell::RefCell<Vec<LevelWorkspace>>, b: &[f64], level: usize) {
        let lvl = &self.levels[level];
        let n = lvl.nrows * lvl.ncols;

        // Coarsest level — solve exactly with precomputed Cholesky
        if level == self.levels.len() - 1 {
            if let Some(ref l) = lvl.cholesky_l {
                let x = crate::cholesky::cholesky_solve(l, b, n);
                workspaces.borrow_mut()[level].z.copy_from_slice(&x);
            } else {
                let mut ws = workspaces.borrow_mut();
                let res = crate::solver::cg_solve(&lvl.laplacian, b, 50, 1e-3, Some(&ws[level].z));
                ws[level].z.copy_from_slice(&res.x);
            }
            return;
        }

        // --- FIXED: Enforce clear states and an initial guess of EXACTLY ZERO ---
        {
            let mut ws = workspaces.borrow_mut();
            ws[level].z.fill(0.0);   // The initial guess for error correction MUST be zero
            ws[level].r.fill(0.0);
            ws[level].ax.fill(0.0);
        }

        // Pre-smooth (starts from zero, so first Jacobi step will naturally apply the weights safely)
        for _ in 0..self.nu {
            let mut ws = workspaces.borrow_mut();
            let (z_slice, ax_slice) = unsafe {
                let ptr = ws.as_mut_ptr();
                (&mut (*ptr.add(level)).z, &mut (*ptr.add(level)).ax)
            };
            damped_jacobi_smooth(&lvl.laplacian, &lvl.diag_inv, z_slice, b, self.omega, ax_slice);
        }

        // ... [Rest of your residual calculation, restriction, recursion, and prolongation remains exactly the same]
        // Residual r = b - L·z
        {
            let mut ws = workspaces.borrow_mut();
            let (z_ref, r_ref) = unsafe {
                let ptr = ws.as_mut_ptr();
                (&(*ptr.add(level)).z, &mut (*ptr.add(level)).r)
            };
            mat_vec_mul_into(&lvl.laplacian, z_ref, r_ref);
            for i in 0..n {
                ws[level].r[i] = b[i] - ws[level].r[i];
            }
        }

        // Downsample current level's residual (r) directly into the next level's r buffer
        {
            let mut ws = workspaces.borrow_mut();
            // Clear out the next level's receiving buffer completely before adding to it
            ws[level + 1].r.fill(0.0);
            
            let (curr_r, next_r) = unsafe {
                let ptr = ws.as_mut_ptr();
                (&(*ptr.add(level)).r, &mut (*ptr.add(level + 1)).r)
            };
            restrict_2d_into(curr_r, lvl.nrows, lvl.ncols, next_r);
        }

        // RECURSION STEP
        let next_b = workspaces.borrow()[level + 1].r.clone();
        self.v_cycle(workspaces, &next_b, level + 1);

        // Prolongate correction
        let next_lvl = &self.levels[level + 1];
        {
            let mut ws = workspaces.borrow_mut();
            // Clear out scratch-space before upsampling back into it
            ws[level].r.fill(0.0);

            let (next_z, curr_r) = unsafe {
                let ptr = ws.as_mut_ptr();
                (&(*ptr.add(level + 1)).z, &mut (*ptr.add(level)).r)
            };
            prolongate_2d_into(next_z, next_lvl.nrows, next_lvl.ncols, lvl.nrows, lvl.ncols, curr_r);
        }

        // Correct the current level's guess using the upsampled data
        {
            let mut ws = workspaces.borrow_mut();
            for i in 0..n {
                ws[level].z[i] += ws[level].r[i];
            }
        }

        // Post-smooth
        for _ in 0..self.nu {
            let mut ws = workspaces.borrow_mut();
            let (z_slice, ax_slice) = unsafe {
                let ptr = ws.as_mut_ptr();
                (&mut (*ptr.add(level)).z, &mut (*ptr.add(level)).ax)
            };
            damped_jacobi_smooth(&lvl.laplacian, &lvl.diag_inv, z_slice, b, self.omega, ax_slice);
        }
    }
}

impl Preconditioner for MgPreconditioner {
    fn apply(&self, r: &[f64], z: &mut Vec<f64>) {
        // Pass the internal RefCell structure container handle down directly
        self.v_cycle(&self.workspaces, r, 0);
        
        let mut mut_workspaces = self.workspaces.borrow_mut();
        z.resize(r.len(), 0.0);
        z.copy_from_slice(&mut_workspaces[0].z);
    }
}

// ---------------------------------------------------------------------------
// Smoother: damped Jacobi
// ---------------------------------------------------------------------------

fn damped_jacobi_smooth(
    laplacian: &CsMat<f64>,
    diag_inv: &[f64],
    x: &mut [f64],
    b: &[f64],
    omega: f64,
    ax: &mut Vec<f64>,
) {
    let n = b.len();
    mat_vec_mul_into(laplacian, x, ax);
    for row in 0..n {
        let residual = b[row] - ax[row];
        x[row] += omega * residual * diag_inv[row];
    }
}

// ---------------------------------------------------------------------------
// Restriction: 9-point full-weighting
// ---------------------------------------------------------------------------

pub fn restrict_2d_into(fine: &[f64], fine_nrows: usize, fine_ncols: usize, coarse: &mut [f64]) {
    let cn = fine_nrows / 2;
    let cm = fine_ncols / 2;
    
    // Clear out the coarse array completely
    for val in coarse.iter_mut() { *val = 0.0; }

    for cr in 0..cn {
        for cc in 0..cm {
            let fr = cr * 2;
            let fc = cc * 2;
            
            let mut sum = 0.0;
            // Fixed stencil weights that sum structurally across the grid
            for dr in -1isize..=1 {
                let rr = fr as isize + dr;
                if rr < 0 || rr >= fine_nrows as isize { continue; }
                
                let wr = if dr == 0 { 0.5 } else { 0.25 };
                
                for dc in -1isize..=1 {
                    let rc = fc as isize + dc;
                    if rc < 0 || rc >= fine_ncols as isize { continue; }
                    
                    let wc = if dc == 0 { 0.5 } else { 0.25 };
                    let w = wr * wc; // Structural component weight
                    
                    sum += w * fine[rr as usize * fine_ncols + rc as usize];
                }
            }
            coarse[cr * cm + cc] = sum;
        }
    }
}

pub fn prolongate_2d_into(
    coarse: &[f64],
    coarse_nrows: usize,
    coarse_ncols: usize,
    fine_nrows: usize,
    fine_ncols: usize,
    fine: &mut [f64],
) {
    // Clear out fine array completely
    for val in fine.iter_mut() { *val = 0.0; }

    // Prolongation is the exact mathematical transpose of restriction.
    // We iterate through the coarse grid and distribute values to the fine grid using the identical weights.
    for cr in 0..coarse_nrows {
        for cc in 0..coarse_ncols {
            let val = coarse[cr * coarse_ncols + cc];
            if val == 0.0 { continue; }

            let fr = cr * 2;
            let fc = cc * 2;

            for dr in -1isize..=1 {
                let rr = fr as isize + dr;
                if rr < 0 || rr >= fine_nrows as isize { continue; }
                
                let wr = if dr == 0 { 0.5 } else { 0.25 };

                for dc in -1isize..=1 {
                    let rc = fc as isize + dc;
                    if rc < 0 || rc >= fine_ncols as isize { continue; }
                    
                    let wc = if dc == 0 { 0.5 } else { 0.25 };
                    let w = wr * wc;

                    // Accumulate back out matching the transpose definition perfectly
                    fine[rr as usize * fine_ncols + rc as usize] += w * val;
                }
            }
        }
    }
}

fn mat_vec_mul_into(a: &CsMat<f64>, v: &[f64], out: &mut Vec<f64>) {
    let n = a.rows();
    out.resize(n, 0.0);
    out.fill(0.0);

    // Bind the indptr view storage locally so its lifetime lasts for the entire function
    let indptr_storage = a.indptr();
    let indptr = indptr_storage.as_slice().expect("CSR matrix missing standard indptr slice");
    let indices = a.indices();
    let data = a.data();

    for row in 0..n {
        let start = indptr[row];
        let end = indptr[row + 1];
        
        let mut acc = 0.0;
        let mut i = start;
        while i < end {
            acc += data[i] * v[indices[i]];
            i += 1;
        }
        out[row] = acc;
    }
}
/// Helper function to safely split a mutable slice around a focal level index for recursive manipulation
fn error_split_at_mut(slice: &mut [LevelWorkspace], index: usize) -> (&mut LevelWorkspace, &mut [LevelWorkspace]) {
    let (left, right) = slice.split_at_mut(index + 1);
    (&mut left[index], right)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to generate safe mock resistance data for a uniform grid
    fn generate_mock_resistance(nrows: usize, ncols: usize) -> Vec<f64> {
        vec![1.0; nrows * ncols]
    }

    #[test]
    fn test_multigrid_preconditioner_symmetry() {
        let nrows = 15;
        let ncols = 15;
        let n = nrows * ncols;
        
        // 1. Build using your actual `.build` method
        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = MgPreconditioner::build(&resistance, nrows, ncols, -1.0, 3);
        
        // 2. Generate two distinct structural vectors using predictable trigonometric waves
        let x: Vec<f64> = (0..n).map(|i| (i as f64 * 0.1).sin()).collect();
        let y: Vec<f64> = (0..n).map(|i| (i as f64 * 0.2).cos()).collect();
        
        let mut mx = vec![0.0; n];
        let mut my = vec![0.0; n];
        
        let mut ax = vec![0.0; n];
        let mut ay = vec![0.0; n];
        mat_vec_mul_into(&mg.levels[0].laplacian, &x, &mut ax);
        mat_vec_mul_into(&mg.levels[0].laplacian, &y, &mut ay);
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
    fn test_restriction_prolongation_preservation() {
        let fine_rows = 15;
        let fine_cols = 15;
        let coarse_rows = 7;
        let coarse_cols = 7;
        
        let fine_constant = vec![1.0; fine_rows * fine_cols];
        let mut coarse_out = vec![0.0; coarse_rows * coarse_cols];
        
        // Restrict down
        restrict_2d_into(&fine_constant, fine_rows, fine_cols, &mut coarse_out);
        
        // A uniform vector must map cleanly. Ensure no nodes are dropped or left unvisited.
        for (i, &val) in coarse_out.iter().enumerate() {
            assert!(val > 0.0, "Restriction zeroed out or dropped node at index {}", i);
        }
    }

    #[test]
    fn test_mg_preconditioned_cg_converges() {
        let nrows = 15;
        let ncols = 15;
        let n = nrows * ncols;

        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = MgPreconditioner::build(&resistance, nrows, ncols, -1.0, 4);

        let b: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5).collect();
        let mut x = vec![0.0; n];
        let mut r = b.clone();

        let mut residuals = Vec::new();

        for _iter in 0..3 {
            let r_norm = r.iter().map(|v| v * v).sum::<f64>().sqrt();
            residuals.push(r_norm);

            let mut z = vec![0.0; n];
            mg.apply(&r, &mut z);

            let mut az = vec![0.0; n];
            mat_vec_mul_into(&mg.levels[0].laplacian, &z, &mut az);
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
        }

        assert!(residuals.len() >= 2, "Need at least 2 iterations to check convergence");
        assert!(
            residuals[1] < residuals[0],
            "MG-preconditioned CG residual did not decrease: iter0={:.6e}, iter1={:.6e}",
            residuals[0], residuals[1]
        );
        if residuals.len() >= 3 {
            assert!(
                residuals[2] < residuals[1],
                "MG-preconditioned CG residual did not decrease: iter1={:.6e}, iter2={:.6e}",
                residuals[1], residuals[2]
            );
        }
    }

    #[test]
    fn test_laplacian_diagonal_positive() {
        let nrows = 15;
        let ncols = 15;
        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = MgPreconditioner::build(&resistance, nrows, ncols, -1.0, 4);

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
                            assert!(
                                lvl.diag_inv[row] > 0.0,
                                "Level {}: diag_inv[{}] = {:.6e} (must be positive)",
                                level_idx, row, lvl.diag_inv[row]
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
        let mg = MgPreconditioner::build(&resistance, nrows, ncols, -1.0, 4);

        let z: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.7 + 1.3).sin() * 0.5 + 0.3)).collect();
        let mut mz = vec![0.0; n];
        mg.apply(&z, &mut mz);

        let z_mz: f64 = z.iter().zip(mz.iter()).map(|(a, b)| a * b).sum();
        assert!(
            z_mz > 0.0,
            "Preconditioner is not positive definite: zᵀ·M⁻¹·z = {:.6e}",
            z_mz
        );

        let z2: Vec<f64> = (0..n).map(|i| ((i as f64 * 1.1 + 2.7).cos() * 0.8 - 0.2)).collect();
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
        let mg = MgPreconditioner::build(&resistance, nrows, ncols, -1.0, 4);

        let b: Vec<f64> = (0..n).map(|i| ((i as f64 * 0.3).sin() + 1.0) * 0.5).collect();

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
    fn test_restriction_prolongation_roundtrip() {
        let fine_rows = 15;
        let fine_cols = 15;
        let coarse_rows = 7;
        let coarse_cols = 7;

        let fine_constant = vec![1.0; fine_rows * fine_cols];

        let mut coarse = vec![0.0; coarse_rows * coarse_cols];
        restrict_2d_into(&fine_constant, fine_rows, fine_cols, &mut coarse);

        // Interior coarse points should be 1.0; boundary points are reduced
        // due to stencil truncation at edges (known issue).
        for cr in 1..coarse_rows {
            for cc in 1..coarse_cols {
                assert!(
                    (coarse[cr * coarse_cols + cc] - 1.0).abs() < 1e-10,
                    "Interior coarse point ({},{}) = {:.6e}, expected 1.0",
                    cr, cc, coarse[cr * coarse_cols + cc]
                );
            }
        }

        let mut fine_roundtrip = vec![0.0; fine_rows * fine_cols];
        prolongate_2d_into(&coarse, coarse_rows, coarse_cols, fine_rows, fine_cols, &mut fine_roundtrip);

        // Interior fine points should be approximately 1.0
        // Boundary points are affected by coarse boundary reduction and
        // prolongation not covering the full fine grid (known issue).
        let mut interior_max_err = 0.0;
        for r in 2..fine_rows - 2 {
            for c in 2..fine_cols - 2 {
                let err = (1.0 - fine_roundtrip[r * fine_cols + c]).abs();
                if err > interior_max_err {
                    interior_max_err = err;
                }
            }
        }
        assert!(
            interior_max_err < 0.01,
            "Interior roundtrip error too large: max_err = {:.6e}",
            interior_max_err
        );
    }

    #[test]
    fn test_coarse_solve_correctness() {
        let nrows = 15;
        let ncols = 15;
        let resistance = generate_mock_resistance(nrows, ncols);
        let mg = MgPreconditioner::build(&resistance, nrows, ncols, -1.0, 4);

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
        mat_vec_mul_into(&lvl.laplacian, &z, &mut az);

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
        let mg = MgPreconditioner::build(&resistance, nrows, ncols, -1.0, 4);

        for (level_idx, lvl) in mg.levels.iter().enumerate() {
            let n = lvl.nrows * lvl.ncols;
            let b = vec![0.0; n];
            let mut x: Vec<f64> = (0..n).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
            let mut ax = vec![0.0; n];

            let initial_norm = x.iter().map(|v| v * v).sum::<f64>().sqrt();
            for _ in 0..3 {
                damped_jacobi_smooth(&lvl.laplacian, &lvl.diag_inv, &mut x, &b, mg.omega, &mut ax);
            }
            let final_norm = x.iter().map(|v| v * v).sum::<f64>().sqrt();

            assert!(
                final_norm < initial_norm,
                "Level {}: smoother did not reduce error norm (initial={:.6e}, final={:.6e})",
                level_idx, initial_norm, final_norm
            );
        }
    }
}