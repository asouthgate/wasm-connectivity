use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Default)]
struct Sample {
    calls: usize,
    elapsed: Duration,
}

#[derive(Default)]
struct Report {
    samples: BTreeMap<(Option<usize>, &'static str), Sample>,
    levels: BTreeMap<usize, (usize, usize, &'static str)>,
    sweeps: BTreeMap<usize, usize>,
    cg_depth: usize,
    outer_iterations: usize,
    fallback_iterations: usize,
}

thread_local! {
    static REPORT: RefCell<Option<Report>> = const { RefCell::new(None) };
}

pub(crate) struct Scope {
    previous: Option<Report>,
}

impl Scope {
    pub(crate) fn new(verbose: bool) -> Self {
        let previous = REPORT.with(|slot| slot.replace(verbose.then(Report::default)));
        Self { previous }
    }

    pub(crate) fn print(&self) {
        REPORT.with(|slot| {
            let report = slot.borrow();
            if let Some(report) = report.as_ref() {
                eprintln!("CGMG timing: {} outer CG iterations; {} coarse fallback CG iterations",
                          report.outer_iterations, report.fallback_iterations);
                eprintln!("Totals for CG solve and preconditioner are inclusive; component rows overlap those totals.");
                eprintln!("Averages are per call; each smoothing call includes all sweeps in that phase.");
                eprintln!("{:<36} {:>10} {:>14} {:>14}", "Step", "Calls", "Total (ms)", "Average (ms)");
                let mut previous_level = None;
                for (&(level, name), sample) in &report.samples {
                    if level != previous_level {
                        if let Some(level) = level {
                            let (nr, nc, repr) = report.levels[&level];
                            if let Some(nu) = report.sweeps.get(&level) {
                                eprintln!("Level {level}: {nr} x {nc}, {repr}, pre/post sweeps: {nu}/{nu}");
                            } else {
                                eprintln!("Level {level}: {nr} x {nc}, {repr}");
                            }
                        }
                        previous_level = level;
                    }
                    let ms = sample.elapsed.as_secs_f64() * 1000.0;
                    eprintln!("{:<36} {:>10} {:>14.6} {:>14.6}", name, sample.calls, ms, ms / sample.calls as f64);
                }

            }
        });
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        REPORT.with(|slot| { slot.replace(self.previous.take()); });
    }
}

pub(crate) struct Timer {
    start: Option<Instant>,
    level: Option<usize>,
    name: &'static str,
}

impl Timer {
    pub(crate) fn exclude(&mut self, diagnostic: impl FnOnce()) {
        let before = self.start.map(|_| Instant::now());
        diagnostic();
        if let (Some(start), Some(before)) = (self.start, before) {
            self.start = start.checked_add(before.elapsed());
        }
    }
}

pub(crate) fn timer(level: Option<usize>, name: &'static str) -> Timer {
    timer_if(true, level, name)
}

pub(crate) fn timer_if(enabled: bool, level: Option<usize>, name: &'static str) -> Timer {
    let active = enabled && REPORT.with(|slot| slot.borrow().is_some());
    Timer { start: active.then(Instant::now), level, name }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            let elapsed = start.elapsed();
            REPORT.with(|slot| {
                if let Some(report) = slot.borrow_mut().as_mut() {
                    let sample = report.samples.entry((self.level, self.name)).or_default();
                    sample.calls += 1;
                    sample.elapsed += elapsed;
                }
            });
        }
    }
}

pub(crate) fn record_duration(level: usize, name: &'static str, elapsed: Duration) {
    REPORT.with(|slot| {
        if let Some(report) = slot.borrow_mut().as_mut() {
            let sample = report.samples.entry((Some(level), name)).or_default();
            sample.calls += 1;
            sample.elapsed += elapsed;
        }
    });
}

pub(crate) fn level(index: usize, nr: usize, nc: usize, repr: &'static str) {
    REPORT.with(|slot| {
        if let Some(report) = slot.borrow_mut().as_mut() {
            report.levels.entry(index).or_insert((nr, nc, repr));
        }
    });
}

pub(crate) fn sweeps(level: usize, nu: usize) {
    REPORT.with(|slot| {
        if let Some(report) = slot.borrow_mut().as_mut() {
            report.sweeps.insert(level, nu);
        }
    });
}

pub(crate) struct CgScope {
    active: bool,
    pub(crate) outer: bool,
}

impl CgScope {
    pub(crate) fn new() -> Self {
        REPORT.with(|slot| {
            let mut slot = slot.borrow_mut();
            if let Some(report) = slot.as_mut() {
                let outer = report.cg_depth == 0;
                report.cg_depth += 1;
                Self { active: true, outer }
            } else {
                Self { active: false, outer: false }
            }
        })
    }

    pub(crate) fn iterations(&self, count: usize) {
        if self.active {
            REPORT.with(|slot| {
                if let Some(report) = slot.borrow_mut().as_mut() {
                    if self.outer { report.outer_iterations += count; }
                    else { report.fallback_iterations += count; }
                }
            });
        }
    }
}

impl Drop for CgScope {
    fn drop(&mut self) {
        if self.active {
            REPORT.with(|slot| {
                if let Some(report) = slot.borrow_mut().as_mut() {
                    report.cg_depth -= 1;
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linalg::multigrid::{MgOptions, MgPreconditioner};
    use crate::linalg::operator::{FineOperator, GroundSpec, StencilOperator};
    use crate::linalg::pcg;

    #[test]
    fn counts_levels_and_scopes_without_leaking() {
        let _scope = Scope::new(true);
        let resistance = vec![1.0; 32 * 32];
        let op = StencilOperator::new(32, 32, -9999.0, &resistance,
                                     GroundSpec::Shunt(vec![1.0; 32 * 32]));
        let mg = MgPreconditioner::build_from_operator(FineOperator::Stencil(op), 32, 32, 8);
        let rhs: Vec<f64> = (0..32 * 32).map(|i| (i as f64).sin()).collect();
        let result = pcg::cg_solve_precond(mg.fine_operator(), &rhs, 100, 1e-6, &mg);
        REPORT.with(|slot| {
            let slot = slot.borrow();
            let report = slot.as_ref().unwrap();
            assert_eq!(report.outer_iterations, result.iters);
            let calls = report.samples[&(None, "Preconditioner (inclusive)")].calls;
            assert_eq!(calls, result.iters);
            assert_eq!(report.levels[&0], (32, 32, "stencil"));
            assert_eq!(report.levels[&1].2, "galerkin-stencil");
            assert_eq!(report.levels[&2].2, "explicit");
            for level in 0..3 {
                for phase in ["Pre-smoothing", "Post-smoothing", "Residual calculation",
                              "Restriction", "Prolongation/correction"] {
                    assert_eq!(report.samples[&(Some(level), phase)].calls, calls);
                }
            }
            assert_eq!(report.samples[&(Some(3), "Coarse direct solve")].calls, calls);
        });
        {
            let _quiet = Scope::new(false);
            assert!(timer(None, "disabled").start.is_none());
            assert!(REPORT.with(|slot| slot.borrow().is_none()));
        }
        assert!(REPORT.with(|slot| slot.borrow().is_some()));
        {
            let _new_solve = Scope::new(true);
            assert!(REPORT.with(|slot| slot.borrow().as_ref().unwrap().samples.is_empty()));
        }
    }

    #[test]
    fn verbose_preserves_both_solver_results_and_zero_iterations() {
        use crate::solve::{self, GroundMode};
        let n = 16 * 16;
        let resistance: Vec<f64> = (0..n).map(|i| 1.0 + (i % 7) as f64).collect();
        let mut source = vec![0.0; n];
        source[0] = 1.0;
        let mut ground = vec![0.0; n];
        ground[n - 1] = 1.0;
        for solve in [solve::solve_raster_sources_mg_with_options,
                      solve::solve_raster_sources_mg_stencil_with_options] {
            let quiet = solve(&resistance, 16, 16, -9999.0, &source, &ground, 1000, 1e-4,
                              false, GroundMode::Shunt, MgOptions::default());
            let verbose = solve(&resistance, 16, 16, -9999.0, &source, &ground, 1000, 1e-4,
                                false, GroundMode::Shunt,
                                MgOptions { verbose: true, ..MgOptions::default() });
            assert_eq!(quiet.total_iters, verbose.total_iters);
            assert_eq!(quiet.output.voltages, verbose.output.voltages);
            assert_eq!(quiet.output.current_map, verbose.output.current_map);
        }
        assert!(REPORT.with(|slot| slot.borrow().is_none()));
        let _scope = Scope::new(true);
        let op = StencilOperator::new(16, 16, -9999.0, &resistance,
                                     GroundSpec::Shunt(vec![1.0; n]));
        let mg = MgPreconditioner::build_with_options(FineOperator::Stencil(op), 16, 16,
            MgOptions { max_levels: 1, cholesky_size: Some(1), verbose: true, ..MgOptions::default() });
        assert_eq!(pcg::cg_solve_precond(mg.fine_operator(), &vec![0.0; n], 100, 1e-4, &mg).iters, 0);
        pcg::cg_solve_precond(mg.fine_operator(), &source, 100, 1e-4, &mg);
        REPORT.with(|slot| {
            let slot = slot.borrow();
            let report = slot.as_ref().unwrap();
            assert!(report.fallback_iterations > 0);
            assert_eq!(report.levels[&0], (16, 16, "stencil"));
            assert!(report.samples.contains_key(&(Some(0), "Coarse fallback CG")));
        });
    }
}
