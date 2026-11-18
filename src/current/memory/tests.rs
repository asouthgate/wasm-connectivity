use sprs::CsMat;
use super::*;
use sprs::TriMat;

fn small_csmat() -> CsMat<f64> {
    let mut t = TriMat::new((3, 3));
    t.add_triplet(0, 0, 2.0);
    t.add_triplet(1, 1, 3.0);
    t.add_triplet(2, 2, 4.0);
    t.add_triplet(0, 1, 1.0);
    t.add_triplet(1, 0, 1.0);
    t.to_csr()
}

#[cfg(feature = "instrumentation-profile")]
#[test]
fn phase_accounting_counts_shared_buffers_once_and_releases_cg_scratch() {
    use crate::current::linalg::multigrid::MgOptions;
    use crate::current::{self, GroundMode};
    let n = 32 * 32;
    let resistance = vec![1.0; n];
    let mut source = vec![0.0; n];
    source[0] = 1.0;
    let mut ground = vec![0.0; n];
    ground[n - 1] = 1.0;
    current::solve_raster_sources_mg_stencil_in_place_with_options(
        &resistance, 32, 32, -9999.0, &mut source, &ground,
        1000, 1e-4, true, GroundMode::Shunt, MgOptions::default(),
    );
    let profile = take_profile().unwrap();
    assert!(profile.rhs_reuses_source_input);
    for phase in &profile.phases {
        let names: std::collections::HashSet<_> = phase.objects.iter().map(|o| &o.name).collect();
        assert_eq!(names.len(), phase.objects.len());
        assert_eq!(phase.total_bytes, phase.objects.iter().map(|o| o.bytes).sum::<u64>());
        for alias in &phase.aliases {
            assert!(!names.contains(&alias.name));
            assert!(names.contains(&alias.allocation));
        }
    }
    let cg = profile.phases.iter().find(|p| p.name == "CG vectors live").unwrap();
    assert!(!cg.objects.iter().any(|o| o.name == "system.rhs" || o.name == "L0.e" || o.name == "L0.d"));
    assert_eq!(cg.objects.iter().filter(|o| o.name.starts_with("cg.")).map(|o| o.bytes).sum::<u64>(), 5 * vec_f64_bytes(n));
    let final_phase = profile.phases.iter().find(|p| p.name == "Final smoothing").unwrap();
    assert_eq!(cg.total_bytes - final_phase.total_bytes, 4 * vec_f64_bytes(n));
    let returned = profile.phases.last().unwrap();
    assert_eq!(returned.total_bytes, 5 * vec_f64_bytes(n));
    for level in &profile.hierarchy {
        assert_eq!(level.laplacian_bytes,
            level.matrix_values_bytes + level.matrix_indices_bytes + level.matrix_indptr_bytes);
        assert_eq!(level.operator_matvec_scratch_bytes,
            level.operator_prolonged_bytes + level.operator_applied_bytes);
    }
    let shared_cg_bytes = cg.total_bytes;
    current::solve_raster_sources_mg_stencil_with_options(
        &resistance, 32, 32, -9999.0, &source, &ground,
        1000, 1e-4, true, GroundMode::Shunt, MgOptions::default(),
    );
    let copied = take_profile().unwrap();
    assert!(!copied.rhs_reuses_source_input);
    let copied_cg = copied.phases.iter().find(|p| p.name == "CG vectors live").unwrap();
    assert_eq!(copied_cg.total_bytes - shared_cg_bytes, vec_f64_bytes(n));
}

#[test]
fn test_vec_sizers() {
    assert_eq!(vec_f64_bytes(10), 80);
    assert_eq!(vec_i32_bytes(10), 40);
    assert_eq!(vec_usize_bytes(10), 10 * usize_size());
    assert_eq!(dense_bytes(4), 4 * 4 * 8);
}

#[test]
fn test_csmat_bytes_formula() {
    let a = small_csmat();
    // rows=3, nnz=5 -> (3+1)*u + 5*u + 5*8
    let expected = (3 + 1) * usize_size() + 5 * usize_size() + 5 * 8;
    assert_eq!(csmat_bytes(&a), expected);
}

#[test]
fn test_triplets_bytes() {
    assert_eq!(triplets_bytes(7), 7 * (2 * usize_size() + 8));
}

#[test]
fn test_take_profile_none_by_default() {
    // A fresh thread (no records) should return None.
    reset();
    assert!(take_profile().is_none());
    #[cfg(not(feature = "instrumentation-profile"))]
    {
        record_cg_vectors(3);
        record_fine_laplacian(&small_csmat());
        assert!(
            take_profile().is_none(),
            "disabled recording must be a no-op"
        );
    }
}

#[cfg(feature = "instrumentation-profile")]
#[test]
fn test_record_accumulates() {
    reset();
    record_conductance_grid(100);
    record_cell_to_node_map(100);
    let a = small_csmat();
    record_fine_laplacian(&a);

    let profile = take_profile().expect("feature enabled -> profile present");
    assert_eq!(profile.conductance_grid_bytes, vec_f64_bytes(100));
    assert_eq!(profile.cell_to_node_map_bytes, vec_i32_bytes(100));
    assert_eq!(profile.laplacian_nodes, 3);
    assert_eq!(profile.laplacian_nnz, 5);
    assert_eq!(profile.laplacian_bytes, csmat_bytes(&a));

    // take() clears the profile
    assert!(take_profile().is_none());
}

#[cfg(feature = "instrumentation-profile")]
#[test]
fn test_record_level_workspace_sizing() {
    reset();
    record_level(1, 64, 9, 1000, 500, 0, GalerkinScratch::default());
    let profile = take_profile().unwrap();
    let lvl = &profile.hierarchy[0];
    assert_eq!(lvl.level, 1);
    assert_eq!(lvl.nodes, 64);
    assert_eq!(lvl.workspace_e_bytes, vec_f64_bytes(64));
    assert_eq!(lvl.workspace_d_bytes, vec_f64_bytes(64));
    assert_eq!(lvl.workspace_d_prime_bytes, vec_f64_bytes(64));
}

#[cfg(feature = "instrumentation-profile")]
#[test]
fn test_record_stencil_fine_zero_bytes() {
    reset();
    use crate::current::linalg::operator::{GroundSpec, StencilOperator};
    let resistance = vec![1.0; 100];
    let mut mask = vec![false; 100];
    mask[0] = true;
    let operator =
        StencilOperator::new(10, 10, -9999.0, &resistance, GroundSpec::Dirichlet(mask));
    record_stencil_fine(&operator);
    record_fine_level(0);
    let profile = take_profile().unwrap();
    assert_eq!(profile.operator_repr, "stencil");
    assert_eq!(profile.laplacian_nodes, 100);
    // 100 diagonal entries + 360 directed edges - 4 edges touching ground.
    assert_eq!(profile.laplacian_nnz, 456);
    assert_eq!(profile.laplacian_bytes, 0);
    assert_eq!(profile.ground_storage_bytes, 100);
    assert_eq!(profile.hierarchy[0].level, 0);
    assert_eq!(profile.hierarchy[0].laplacian_bytes, 0);
    assert_eq!(profile.hierarchy[0].workspace_e_bytes, 0);
    assert_eq!(profile.hierarchy[0].workspace_d_bytes, 0);
    assert_eq!(profile.hierarchy[0].workspace_d_prime_bytes, 800);
}

#[cfg(feature = "instrumentation-profile")]
#[test]
fn test_record_fine_laplacian_explicit_repr() {
    reset();
    let a = small_csmat();
    record_fine_laplacian(&a);
    record_fine_level(0);
    let profile = take_profile().unwrap();
    assert_eq!(profile.operator_repr, "explicit");
    assert_eq!(profile.hierarchy[0].level, 0);
    assert_eq!(profile.hierarchy[0].laplacian_bytes, csmat_bytes(&a));
}
