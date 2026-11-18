use super::{AnnotatedOutput, GroundMode, RasterOutput};
use crate::linalg::real::{Real, scalar};
use crate::linalg::multigrid::{MgOptions, MgPreconditioner};
use crate::memory;
use crate::linalg::timing;
use super::ground::build_dirichlet_mask_identity;
use super::output::build_raster_output_identity;
use super::residual::final_stencil_residual_error;
use super::sources::{balance_sources, valid_positive_value};
use crate::linalg::operator::{FineOperator, GroundSpec, StencilOperator};

fn prepare_stencil_sources<'a, F: Real>(
    source_data: &mut [F],
    ground_data: &'a [F],
    num_nodes: usize,
    nodata: F,
    remove_average: bool,
    ground_mode: GroundMode,
) -> GroundSpec<'a, F> {
    for value in source_data.iter_mut() {
        if !valid_positive_value(*value, nodata) {
            *value = scalar::<F>(0.0);
        }
    }

    let (ground_spec, grounds_present) = match ground_mode {
        GroundMode::Shunt => {
            let grounds_present = ground_data.iter().any(|&value| valid_positive_value(value, nodata));
            (GroundSpec::ShuntRaster(ground_data), grounds_present)
        }
        GroundMode::Dirichlet => {
            let mask = build_dirichlet_mask_identity(ground_data, nodata);
            let grounds_present = mask.iter().any(|&pinned| pinned);
            for index in 0..num_nodes {
                if mask[index] {
                    source_data[index] = scalar::<F>(0.0);
                }
            }
            (GroundSpec::Dirichlet(mask), grounds_present)
        }
    };

    balance_sources(source_data, remove_average && !grounds_present);
    ground_spec
}

/// Prepare the caller's source buffer in place (WASM boundary).
pub fn solve_raster_sources_mg_stencil_in_place_with_options<F: Real>(
    resistance_data: &[F],
    nrows: usize,
    ncols: usize,
    nodata: F,
    source_data: &mut [F],
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
    memory::reset();
    let num_nodes = nrows * ncols;
    memory::record_inputs_typed::<F>(num_nodes, true);

    let ground_spec = prepare_stencil_sources(
        source_data, ground_data, num_nodes, nodata, remove_average, ground_mode,
    );

    let op = StencilOperator::new(nrows, ncols, nodata, resistance_data, ground_spec);

    memory::record_cg_vectors_typed::<F>(num_nodes);

    drop(phase_timer);
    let phase_timer = timing::timer(None, "Hierarchy construction");
    let mg = MgPreconditioner::build_with_options(FineOperator::Stencil(op), nrows, ncols, options);
    drop(phase_timer);
    let phase_timer = timing::timer(None, "CG solve (inclusive)");
    let mut res = mg.solve_cg(source_data, max_iter, tol, options.profile);

    drop(phase_timer);
    mg.final_post_smooth(source_data, &mut res.v, options.l0postnu, options.profile, res.iters);
    let error = final_stencil_residual_error(mg.fine_stencil(), source_data, &res.v, tol);
    let phase_timer = timing::timer(None, "Current map/output construction");
    let out = build_raster_output_identity(
        res.v,
        resistance_data,
        ground_data,
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

/// Native variant accepting multigrid setup controls; browser calls use defaults.
pub fn solve_raster_sources_mg_stencil_with_options<F: Real>(
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
    let mut source = source_data.to_vec();
    let result = solve_raster_sources_mg_stencil_in_place_with_options(
        resistance_data, nrows, ncols, nodata, &mut source, ground_data,
        max_iter, tol, remove_average, ground_mode, options,
    );
    memory::record_source_copy();
    result
}

/// Matrix-free (low-memory) MG solve: the retained fine Laplacian is a stencil
/// over the input resistance raster; level 1 reconstructs Galerkin rows on
/// demand during setup and level 2 is the first CSR. Matvecs compose transfers
/// with the fine operator; nonterminal levels use weighted Jacobi smoothing.
/// All transfers are geometric stencils.
/// A terminal matrix-free level can use a temporary dense direct solve.
pub fn solve_raster_sources_mg_stencil<F: Real>(
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
    solve_raster_sources_mg_stencil_with_options(
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
