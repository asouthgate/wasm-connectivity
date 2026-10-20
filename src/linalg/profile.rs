use std::cell::RefCell;
use crate::linalg::operator::Operator;

#[derive(Default)]
struct Trace {
    iteration: usize,
    previous: Vec<Option<f64>>,
    weighted_previous: Vec<Option<f64>>,
    weighted_global_previous: Option<f64>,
    global_previous: Option<f64>,
    configured: Vec<bool>,
    #[cfg(test)]
    events: Vec<Event>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct Event {
    iteration: usize,
    level: Option<usize>,
    phase: &'static str,
    sweep: Option<(usize, usize)>,
    absolute: f64,
    relative: f64,
    ratio: Option<f64>,
    weighted: f64,
    weighted_ratio: Option<f64>,
}

thread_local! {
    static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) };
}

pub(crate) struct Scope {
    previous: Option<Trace>,
}

impl Scope {
    pub(crate) fn new(enabled: bool) -> Self {
        let previous = TRACE.with(|slot| slot.replace(enabled.then(Trace::default)));
        if enabled {
            eprintln!("PROFILE equation residuals: abs=||d-Ae||_2, rel=abs/||d||_2; weighted=||D^-1/2(d-Ae)||_2; ratios compare the preceding sample on the same level.");
            eprintln!("PROFILE {:>6} {:>5} {:<26} {:>14} {:>14} {:>12} {:>14} {:>12}",
                "CG iter", "Level", "Step", "Abs residual", "Rel residual", "Ratio", "Weighted", "W ratio");
        }
        Self { previous }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        TRACE.with(|slot| { slot.replace(self.previous.take()); });
    }
}

pub(crate) fn iteration(iteration: usize) {
    TRACE.with(|slot| {
        slot.borrow_mut().as_mut().expect("profiling scope required").iteration = iteration;
    });
}

fn divide(value: f64, denominator: f64) -> f64 {
    if denominator == 0.0 {
        if value == 0.0 { 0.0 } else { f64::INFINITY }
    } else {
        value / denominator
    }
}

#[cfg(test)]
fn measure(op: &dyn Operator, rhs: &[f64], x: &[f64]) -> (f64, f64) {
    let (absolute, relative, _) = measure_weighted(op, rhs, x);
    (absolute, relative)
}

fn measure_weighted(op: &dyn Operator, rhs: &[f64], x: &[f64]) -> (f64, f64, f64) {
    let mut applied = vec![0.0; op.n()];
    op.matvec(x, &mut applied);
    let diagonal = op.diag_inv();
    let mut weighted = 0.0_f64;
    let mut residual = 0.0_f64;
    let mut rhs_norm = 0.0_f64;
    for ((&d, &ae), &inverse) in rhs.iter().zip(&applied).zip(&diagonal) {
        weighted = weighted.hypot((d - ae) * inverse.abs().sqrt());
        residual = residual.hypot(d - ae);
        rhs_norm = rhs_norm.hypot(d);
    }
    (residual, divide(residual, rhs_norm), weighted)
}

impl Trace {
    fn record(&mut self, level: Option<usize>, phase: &'static str,
        sweep: Option<(usize, usize)>, absolute: f64, relative: f64, weighted: f64, entry: bool)
    {
        let previous = if let Some(level) = level {
            if self.previous.len() <= level { self.previous.resize(level + 1, None); }
            if entry { self.previous[level] = None; }
            &mut self.previous[level]
        } else {
            &mut self.global_previous
        };
        let ratio = previous.map(|old| {
            if old == 0.0 && absolute == 0.0 { 1.0 } else { divide(absolute, old) }
        });
        *previous = Some(absolute);
        let weighted_previous = if let Some(level) = level {
            if self.weighted_previous.len() <= level { self.weighted_previous.resize(level + 1, None); }
            if entry { self.weighted_previous[level] = None; }
            &mut self.weighted_previous[level]
        } else { &mut self.weighted_global_previous };
        let weighted_ratio = weighted_previous.map(|old| {
            if old == 0.0 && weighted == 0.0 { 1.0 } else { divide(weighted, old) }
        });
        *weighted_previous = Some(weighted);
        let weighted_ratio_label = weighted_ratio.map_or_else(|| "-".to_owned(), |r| format!("{r:.6e}"));
        let level_label = level.map_or_else(|| "CG".to_owned(), |level| level.to_string());
        let step = match sweep {
            Some((index, count)) => format!("{phase} {index}/{count}"),
            None => phase.to_owned(),
        };
        let ratio_label = ratio.map_or_else(|| "-".to_owned(), |ratio| format!("{ratio:.6e}"));
        eprintln!("PROFILE {:>7} {:>5} {:<26} {:>14.6e} {:>14.6e} {:>12} {:>14.6e} {:>12}",
            self.iteration, level_label, step, absolute, relative, ratio_label, weighted, weighted_ratio_label);
        #[cfg(test)]
        self.events.push(Event { iteration: self.iteration, level, phase, sweep,
            absolute, relative, ratio, weighted, weighted_ratio });
    }
}

pub(crate) fn level_sample(level: usize, phase: &'static str, sweep: Option<(usize, usize)>,
    op: &dyn Operator, rhs: &[f64], x: &[f64], entry: bool)
{
    let (absolute, relative, weighted) = measure_weighted(op, rhs, x);
    TRACE.with(|slot| {
        slot.borrow_mut().as_mut().expect("profiling scope required")
            .record(Some(level), phase, sweep, absolute, relative, weighted, entry);
    });
}

pub(crate) fn smoother_info(level: usize, op: &dyn Operator, cached: Option<(&[f64], f64)>) {
    let first = TRACE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let trace = slot.as_mut().expect("profiling scope required");
        if trace.configured.len() <= level { trace.configured.resize(level + 1, false); }
        let first = !trace.configured[level];
        trace.configured[level] = true;
        first
    });
    if !first { return; }
    if let Some((cached, omega)) = cached {
        let actual = op.diag_inv();
        let mismatch = cached.iter().zip(actual).fold(0.0_f64, |max, (&a, b)| {
            max.max(divide((a - b).abs(), a.abs().max(b.abs())))
        });
        eprintln!("PROFILE level {level}: Jacobi omega={omega:.9e}, max relative cached diagonal mismatch={mismatch:.6e}");
    } else {
        eprintln!("PROFILE level {level}: Gauss-Seidel or terminal solve");
    }
}

pub(crate) fn global_sample(iteration: usize, phase: &'static str,
    op: &dyn Operator, rhs: &[f64], x: &[f64])
{
    global_sweep_sample(iteration, phase, None, op, rhs, x);
}

pub(crate) fn global_sweep_sample(iteration: usize, phase: &'static str, sweep: Option<(usize, usize)>,
    op: &dyn Operator, rhs: &[f64], x: &[f64])
{
    let (absolute, relative, weighted) = measure_weighted(op, rhs, x);
    TRACE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let trace = slot.as_mut().expect("profiling scope required");
        trace.iteration = iteration;
        trace.record(None, phase, sweep, absolute, relative, weighted, false);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use crate::linalg::{pcg, timing};
    use crate::linalg::multigrid::{MgOptions, MgPreconditioner};
    use crate::linalg::operator::{FineOperator, GroundSpec, StencilOperator};

    fn events() -> Vec<Event> {
        TRACE.with(|slot| slot.borrow().as_ref().unwrap().events.clone())
    }

    #[test]
    fn weighted_residual_distinguishes_transient_euclidean_growth() {
        let mut entries = sprs::TriMat::new((2, 2));
        for (i, j, v) in [(0,0,1.0), (0,1,9.0), (1,0,9.0), (1,1,100.0)] {
            entries.add_triplet(i, j, v);
        }
        let op = entries.to_csr::<usize>();
        let _scope = Scope::new(true);
        level_sample(0, "entry", None, &op, &[1.0, 0.0], &[0.0, 0.0], true);
        level_sample(0, "pre-sweep", Some((1,1)), &op, &[1.0, 0.0], &[0.8, 0.0], false);
        let rows = events();
        assert!(rows[1].ratio.unwrap() > 7.0);
        assert!(rows[1].weighted_ratio.unwrap() < 1.0);
        assert!((rows[1].weighted - (0.04_f64 + 0.5184).sqrt()).abs() < 1e-14);
    }

    #[test]
    fn residual_values_ratios_and_scope_isolation() {
        let mut entries = sprs::TriMat::new((2, 2));
        entries.add_triplet(0, 0, 1.0);
        entries.add_triplet(1, 1, 1.0);
        let op = entries.to_csr::<usize>();
        assert_eq!(measure(&op, &[1.0, 0.0], &[0.0, 1.0]), (2.0_f64.sqrt(), 2.0_f64.sqrt()));
        assert_eq!(divide(0.0, 0.0), 0.0);
        assert_eq!(divide(1.0, 0.0), f64::INFINITY);
        let _scope = Scope::new(true);
        iteration(3);
        level_sample(0, "entry", None, &op, &[1.0, 0.0], &[0.0, 0.0], true);
        level_sample(1, "entry", None, &op, &[100.0, 0.0], &[0.0, 0.0], true);
        level_sample(0, "pre-sweep", Some((1, 2)), &op, &[1.0, 0.0], &[-1.0, 0.0], false);
        level_sample(0, "post-sweep", Some((1, 1)), &op, &[1.0, 0.0], &[1.0, 0.0], false);
        level_sample(0, "post-sweep", Some((1, 1)), &op, &[1.0, 0.0], &[1.0, 0.0], false);
        level_sample(0, "entry", None, &op, &[2.0, 0.0], &[0.0, 0.0], true);
        let rows = events();
        assert_eq!(rows[2].iteration, 3);
        assert_eq!(rows[2].absolute, 2.0);
        assert_eq!(rows[2].relative, 2.0);
        assert_eq!(rows[2].ratio, Some(2.0));
        assert_eq!(rows[3].ratio, Some(0.0));
        assert_eq!(rows[4].ratio, Some(1.0));
        assert_eq!(rows[5].ratio, None);
        {
            let _quiet = Scope::new(false);
            assert!(TRACE.with(|slot| slot.borrow().is_none()));
        }
        assert_eq!(events().len(), 6);
    }

    #[test]
    fn v_cycle_trace_order_and_results_for_both_representations() {
        let n = 32 * 32;
        let resistance: Vec<_> = (0..n).map(|i| 1.0 + (i % 13) as f64).collect();
        let rhs: Vec<_> = (0..n).map(|i| (i as f64 * 0.17).sin()).collect();
        for low_memory in [false, true] {
            let fine = if low_memory {
                FineOperator::Stencil(StencilOperator::new(32, 32, -9999.0, &resistance,
                    GroundSpec::Shunt(vec![1.0; n])))
            } else {
                let (_, _, _, matrix) = crate::build_circuit_model(&resistance, 32, 32, -9999.0);
                FineOperator::Explicit(crate::circuit::laplacian::add_diagonal(&matrix, &vec![1.0; n]))
            };
            let mg = MgPreconditioner::build_with_options(fine, 32, 32,
                MgOptions::default());
            let _scope = Scope::new(true);
            // An active trace cannot cause ordinary execution to profile.
            let quiet = mg.solve_cg(&rhs, 1000, 1e-8, false);
            assert!(events().is_empty());
            let profiled = mg.solve_cg(&rhs, 1000, 1e-8, true);
            assert_eq!(quiet.iters, profiled.iters);
            assert_eq!(quiet.v, profiled.v);
            let rows = events();
            assert_eq!(rows[0].iteration, 0);
            assert_eq!(rows[0].phase, "CG initial");
            assert_eq!(rows[0].level, None);
            let mut expected = Vec::new();
            for (level, nu) in [2, 5, 25].into_iter().enumerate() {
                expected.push((Some(level), "entry", None));
                for sweep in 1..=nu {
                    expected.push((Some(level), "pre-sweep", Some((sweep, nu))));
                }
            }
            expected.push((Some(3), "entry", None));
            expected.push((Some(3), "terminal solve", None));
            for (level, nu) in [(2, 25), (1, 5), (0, 2)] {
                expected.push((Some(level), "coarse correction", None));
                for sweep in 1..=nu {
                    expected.push((Some(level), "post-sweep", Some((sweep, nu))));
                }
            }
            expected.push((None, "CG update", None));
            let actual: Vec<_> = rows.iter().filter(|row| row.iteration == 1)
                .map(|row| (row.level, row.phase, row.sweep)).collect();
            assert_eq!(actual, expected);
            for row in rows.iter().filter(|row| row.phase == "entry") {
                assert_eq!(row.ratio, None);
            }
            let last = rows.last().unwrap();
            assert_eq!(last.phase, "CG update");
            assert_eq!(last.iteration, profiled.iters);
            assert!(last.relative < 1e-8);
            // No V-cycle should run for a zero right-hand side.
            let _zero = Scope::new(true);
            assert_eq!(mg.solve_cg(&vec![0.0; n], 100, 1e-8, true).iters, 0);
            let rows = events();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].absolute, 0.0);
            assert_eq!(rows[0].relative, 0.0);
        }
    }

    #[test]
    fn solve_flags_preserve_outputs_and_terminal_fallback() {
        use crate::solve::{self, GroundMode};
        let n = 16 * 16;
        let resistance: Vec<_> = (0..n).map(|i| 1.0 + (i % 7) as f64).collect();
        let mut source = vec![0.0; n];
        source[0] = 1.0;
        let ground = vec![1.0; n];
        for solve in [solve::solve_raster_sources_mg_with_options,
                      solve::solve_raster_sources_mg_stencil_with_options] {
            for max_levels in [1, 8] {
                let base = MgOptions { max_levels, l0postnu: 5, cholesky_size: Some(1), ..MgOptions::default() };
                let quiet = solve(&resistance, 16, 16, -9999.0, &source, &ground,
                    1000, 1e-6, false, GroundMode::Shunt, base);
                for (profile, verbose) in [(true, false), (false, true), (true, true)] {
                    let traced = solve(&resistance, 16, 16, -9999.0, &source, &ground,
                        1000, 1e-6, false, GroundMode::Shunt,
                        MgOptions { profile, verbose, ..base });
                    assert_eq!(quiet.total_iters, traced.total_iters);
                    assert_eq!(quiet.output.voltages, traced.output.voltages);
                    assert_eq!(quiet.output.current_map, traced.output.current_map);
                    assert!(TRACE.with(|slot| slot.borrow().is_none()));
                }
            }
        }
    }

    struct CountedOperator {
        matvecs: Cell<usize>,
        rows: Cell<usize>,
    }

    impl Operator for CountedOperator {
        fn n(&self) -> usize { 2 }
        fn matvec(&self, x: &[f64], y: &mut [f64]) {
            self.matvecs.set(self.matvecs.get() + 1);
            y[0] = 2.0 * x[0];
            y[1] = 3.0 * x[1];
        }
        fn diag_inv(&self) -> Vec<f64> { vec![0.5, 1.0 / 3.0] }
        fn for_each_entry(&self, row: usize, f: &mut dyn FnMut(usize, f64)) {
            self.rows.set(self.rows.get() + 1);
            f(row, if row == 0 { 2.0 } else { 3.0 });
        }
    }

    #[test]
    fn ordinary_and_verbose_cg_do_no_diagnostic_operator_work() {
        for verbose in [false, true] {
            let _timing = timing::Scope::new(verbose);
            let _scope = Scope::new(true);
            let op = CountedOperator { matvecs: Cell::new(0), rows: Cell::new(0) };
            let preconditioner = pcg::JacobiPreconditioner::new(&op);
            let quiet = pcg::cg_solve_precond(&op, &[1.0, 2.0], 100, 1e-10, &preconditioner);
            assert_eq!(op.matvecs.get(), quiet.iters);
            assert_eq!(op.rows.get(), 0);
            assert!(events().is_empty());
            op.matvecs.set(0);
            let profiled = pcg::cg_solve_precond_profiled(&op, &[1.0, 2.0], 100, 1e-10, &preconditioner);
            assert_eq!(quiet.iters, profiled.iters);
            assert_eq!(quiet.v, profiled.v);
            assert_eq!(op.matvecs.get(), 2 * profiled.iters + 1);
            assert_eq!(op.rows.get(), 0);
            assert_eq!(events().len(), profiled.iters + 1);
        }
    }
}
