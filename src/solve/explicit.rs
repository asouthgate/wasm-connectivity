use super::{AnnotatedOutput, GroundMode, RasterOutput};
use crate::linalg::real::{Real, scalar};
use crate::linalg::multigrid::{MgOptions, MgPreconditioner};
use crate::memory;
use crate::linalg::timing;
use super::ground::{apply_dirichlet_ground_lap, collect_ground_nodes, zero_ground_rhs};
use super::output::build_raster_output;
use super::residual::final_residual_error;
use super::sources::{balance_sources, build_node_values};
use crate::linalg::operator::FineOperator;
use sprs::CsMat;

struct ExplicitSystem<F: Real> {
    laplacian: CsMat<F>,
    source_by_node: Vec<F>,
    cell_to_node: Vec<i32>,
}

fn prepare_explicit_system<F: Real>(
    resistance_data: &[F],
    nrows: usize,
    ncols: usize,
    nodata: F,
    source_data: &[F],
    ground_data: &[F],
    remove_average: bool,
    ground_mode: GroundMode,
) -> ExplicitSystem<F> {
    memory::reset();
    memory::record_inputs_typed::<F>(nrows * ncols, false);
    let filled = crate::raster::fill_nodata(resistance_data, nodata);
    memory::record_filled_resistance_typed::<F>(filled.len());

    let (cell_to_node, num_nodes, mut laplacian) =
        crate::circuit::build_circuit_matrix(&filled, nrows, ncols, nodata);
    drop(filled);
    let current_global =
        build_node_values(&cell_to_node, num_nodes, nrows, ncols, nodata, source_data);

    let grounds_present;
    let (mut laplacian, mut s) = match ground_mode {
        GroundMode::Shunt => {
            let g =
                build_node_values(&cell_to_node, num_nodes, nrows, ncols, nodata, ground_data);
            memory::record_ground_setup(memory::vec_scalar_bytes::<F>(g.len()));
            grounds_present = g.iter().any(|&x| x > scalar::<F>(0.0));
            if grounds_present {
                crate::circuit::laplacian::add_diagonal_in_place(&mut laplacian, &g);
            }
            (laplacian, current_global)
        }
        GroundMode::Dirichlet => {
            let gns = collect_ground_nodes(&cell_to_node, ground_data, nrows, ncols, nodata);
            memory::record_ground_setup(memory::vec_usize_bytes(gns.len()));
            grounds_present = !gns.is_empty();
            let laplacian = if grounds_present {
                apply_dirichlet_ground_lap(&laplacian, &gns)
            } else {
                laplacian
            };
            let mut s = current_global;
            zero_ground_rhs(&mut s, &gns);
            (laplacian, s)
        }
    };

    if !grounds_present {
        crate::circuit::laplacian::regularize_laplacian(&mut laplacian);
    }

    balance_sources(&mut s, remove_average && !grounds_present);

    memory::record_fine_laplacian(&laplacian);
    memory::record_cg_vectors_typed::<F>(num_nodes);

    ExplicitSystem { laplacian, source_by_node: s, cell_to_node }
}

/// Native variant accepting multigrid setup controls; browser calls use defaults.
pub fn solve_raster_sources_mg_with_options<F: Real>(
    resistance_data: &[F],
    nrows: usize,
    ncols: usize,
    nodata: F,
    source_data: &[F],
    ground_data: &[F],
    max_iter: usize,
    tol: F,
    remove_average: bool,
    ground_mode: GroundMode,
    options: MgOptions,
) -> AnnotatedOutput<RasterOutput<F>> {
    let _profile_scope = crate::linalg::profile::Scope::new(options.profile);
    let timing_scope = timing::Scope::new(options.verbose);
    let phase_timer = timing::timer(None, "System preparation");
    let ExplicitSystem { laplacian, source_by_node, cell_to_node } = prepare_explicit_system(
        resistance_data, nrows, ncols, nodata, source_data, ground_data,
        remove_average, ground_mode,
    );

    drop(phase_timer);
    let phase_timer = timing::timer(None, "Hierarchy construction");
    let mg = MgPreconditioner::build_with_options(
        FineOperator::Explicit(laplacian),
        nrows,
        ncols,
        options,
    );

    drop(phase_timer);
    let phase_timer = timing::timer(None, "CG solve (inclusive)");
    let mut res = mg.solve_cg(&source_by_node, max_iter, tol, options.profile);

    drop(phase_timer);
    mg.final_post_smooth(&source_by_node, &mut res.v, options.l0postnu, options.profile, res.iters);
    let error = final_residual_error(mg.fine_operator(), &source_by_node, &res.v, tol);
    let phase_timer = timing::timer(None, "Current map/output construction");
    let out = build_raster_output(
        &res.v,
        resistance_data,
        ground_data,
        &cell_to_node,
        nrows,
        ncols,
        nodata,
        ground_mode,
    );

    drop(phase_timer);
    timing_scope.print();

    AnnotatedOutput {
        output: out,
        total_iters: res.iters,
        error,
    }
}

/// Multigrid-preconditioned raster-mode solve. Fills nodata cells with a
/// high resistance so the domain is a perfect rectangle, builds a
/// geometric multigrid hierarchy, and uses MG V-cycles as the CG
/// preconditioner.  Returns the output plus total PCG iteration count.
///
/// This solves the full-grid system directly (no component extraction) because
/// the MG hierarchy is built on the full rectangular grid and its
/// restriction/prolongation operators
/// assume raster-order indexing.
pub fn solve_raster_sources_mg<F: Real>(
    resistance_data: &[F],
    nrows: usize,
    ncols: usize,
    nodata: F,
    source_data: &[F],
    ground_data: &[F],
    max_iter: usize,
    tol: F,
    remove_average: bool,
    ground_mode: GroundMode,
) -> AnnotatedOutput<RasterOutput<F>> {
    solve_raster_sources_mg_with_options(
        resistance_data,
        nrows,
        ncols,
        nodata,
        source_data,
        ground_data,
        max_iter,
        tol,
        remove_average,
        ground_mode,
        MgOptions::default(),
    )
}
