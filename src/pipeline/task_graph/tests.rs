use crate::pipeline::{
    CurrentSolver, ExecutionOptions, Grid, NodeResult, OutputLayer, PipelineData, PipelineGraph,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

fn grid() -> Grid {
    Grid {
        nrows: 1,
        ncols: 3,
        pixel_width: 1.0,
    }
}

fn options(threads: usize) -> ExecutionOptions {
    ExecutionOptions {
        samples: 3,
        threads,
        seed: 42,
        all_layers: true,
        statistics: true,
    }
}

fn model(draw: serde_json::Value) -> PipelineGraph {
    PipelineGraph::from_json(
        &json!({
            "name":"sampling", "inputs":{"data":{"type":"raster_ranked"}},
            "nodes":{
                "fixed":{"op":"round","inputs":"inputs.data"},
                "resistance":{"op":"power_response","inputs":"nodes.fixed","parameters":{
                    "input_range":[0,4], "base_range":[0,4], "exponent":draw, "scale":1, "offset":1
                }}
            }, "output":"nodes.resistance"
        })
        .to_string(),
    )
    .unwrap()
}

fn run(
    graph: &PipelineGraph,
    options: ExecutionOptions,
    current_solver: Option<&CurrentSolver<'_>>,
) -> Vec<NodeResult> {
    let mut results = Vec::new();
    graph
        .execute(
            HashMap::from([("data".into(), PipelineData::raster(vec![1.0, 2.0, 4.0]))]),
            grid(),
            options,
            current_solver,
            |result| {
                results.push(result);
                Ok(())
            },
        )
        .unwrap();
    results
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-10 * expected.abs().max(1.0),
        "{actual} != {expected}"
    );
}

#[test]
fn threaded_layer_and_current_summaries_match_individual_samples() {
    let graph = model(json!({"sample":{"values":[1,2,3]}}));
    let source = [1.0, 0.0, 0.0];
    let ground = [0.0, 1.0, 1.0];
    let mut solver = CurrentSolver::new(&source, &ground, Default::default());
    solver.settings.explicit = true;
    solver.settings.ground_mode = crate::solve::GroundMode::Dirichlet;
    solver.settings.tolerance = 1e-8;
    for threads in [1, 2, 8] {
        let results = run(&graph, options(threads), Some(&solver));
        for layer in [
            OutputLayer::Resistance,
            OutputLayer::Current,
            OutputLayer::Intermediate {
                node_name: "fixed".into(),
                port_name: "data".into(),
            },
        ] {
            let samples: Vec<_> = results
                .iter()
                .filter(|result| result.layer == layer && result.sample.is_some())
                .collect();
            assert_eq!(samples.len(), 3);
            let summary = results
                .iter()
                .find(|result| result.layer == layer && result.sample.is_none())
                .unwrap();
            for cell in 0..3 {
                let values: Vec<_> = samples
                    .iter()
                    .map(|sample| sample.outputs["data"].values().unwrap()[cell])
                    .collect();
                let mean = values.iter().sum::<f64>() / 3.0;
                let variance = values
                    .iter()
                    .map(|value| (value - mean).powi(2))
                    .sum::<f64>()
                    / 2.0;
                close(summary.outputs["mean"].values().unwrap()[cell], mean);
                close(
                    summary.outputs["variance"].values().unwrap()[cell],
                    variance,
                );
            }
        }
        for result in results
            .iter()
            .filter(|result| result.layer == OutputLayer::Resistance && result.sample.is_some())
        {
            let expected = solver
                .solve_current(result.outputs["data"].values().unwrap(), 1, 3)
                .unwrap();
            let current = results
                .iter()
                .find(|current| {
                    current.layer == OutputLayer::Current && current.sample == result.sample
                })
                .unwrap();
            assert_eq!(current.outputs["data"].values().unwrap(), expected);
        }
        let fixed: Vec<_> = results
            .iter()
            .filter(|result| {
                matches!(result.layer, OutputLayer::Intermediate { .. }) && result.sample.is_some()
            })
            .collect();
        let PipelineData::Raster(first) = &fixed[0].outputs["data"] else {
            panic!()
        };
        for result in fixed.iter().skip(1) {
            let PipelineData::Raster(values) = &result.outputs["data"] else {
                panic!()
            };
            assert!(Arc::ptr_eq(first, values));
        }
    }
}

#[test]
fn seeded_draws_do_not_depend_on_thread_count() {
    for distribution in [json!({"uniform":[1,3]}), json!({"log_uniform":[1,3]})] {
        let graph = model(json!({"sample":distribution}));
        let sequential = run(&graph, options(1), None);
        let parallel = run(&graph, options(3), None);
        for expected in sequential.iter().filter(|result| result.sample.is_some()) {
            let actual = parallel
                .iter()
                .find(|result| result.layer == expected.layer && result.sample == expected.sample)
                .unwrap();
            assert_eq!(
                actual.outputs["data"].values().unwrap(),
                expected.outputs["data"].values().unwrap()
            );
        }
        let other_seed = run(
            &graph,
            ExecutionOptions {
                seed: 99,
                ..options(2)
            },
            None,
        );
        assert_ne!(
            sequential
                .iter()
                .find(|result| result.layer == OutputLayer::Resistance && result.sample == Some(0))
                .unwrap()
                .outputs["data"]
                .values()
                .unwrap(),
            other_seed
                .iter()
                .find(|result| result.layer == OutputLayer::Resistance && result.sample == Some(0))
                .unwrap()
                .outputs["data"]
                .values()
                .unwrap()
        );
    }
}

#[test]
fn errors_stop_execution_without_replacing_invalid_samples() {
    let graph = model(json!({"sample":{"values":[1,-1,2]}}));
    let inputs = || HashMap::from([("data".into(), PipelineData::raster(vec![1.0, 2.0, 4.0]))]);
    let error = graph
        .execute(inputs(), grid(), options(3), None, |_| Ok(()))
        .unwrap_err();
    assert!(error.contains("sample 1, node resistance"));
    let graph = model(json!({"sample":{"values":[1,2,3]}}));
    let error = graph
        .execute(inputs(), grid(), options(3), None, |_| {
            Err("write failed".into())
        })
        .unwrap_err();
    assert!(error.contains("write failed"));
    for invalid in [
        ExecutionOptions {
            threads: 0,
            ..options(1)
        },
        ExecutionOptions {
            samples: 0,
            ..options(1)
        },
        ExecutionOptions {
            samples: 1,
            ..options(1)
        },
        ExecutionOptions {
            samples: 2,
            ..options(1)
        },
    ] {
        assert!(graph
            .execute(inputs(), grid(), invalid, None, |_| Ok(()))
            .is_err());
    }
}

#[test]
fn ordinary_runs_preserve_infinity_without_allocating_statistics() {
    let graph = PipelineGraph::from_json(
        &json!({
            "name":"distance", "inputs":{"data":{"type":"raster_ranked"}},
            "nodes":{"distance":{"op":"distance","inputs":"inputs.data","parameters":{
                "units":"metres", "when_empty":"infinity"
            }}}, "output":"nodes.distance"
        })
        .to_string(),
    )
    .unwrap();
    let mut count = 0;
    graph
        .execute(
            HashMap::from([("data".into(), PipelineData::raster(vec![0.0; 3]))]),
            grid(),
            ExecutionOptions {
                samples: 1,
                statistics: false,
                all_layers: false,
                ..options(2)
            },
            None,
            |result| {
                assert_eq!(result.sample, Some(0));
                assert!(result.outputs["data"]
                    .values()?
                    .iter()
                    .all(|value| value.is_infinite()));
                count += 1;
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn current_operation_can_feed_another_calculation() {
    let graph = PipelineGraph::from_json(&json!({
            "name":"current", "inputs":{
                "resistance":{"type":"raster_ranked"}, "source":{"type":"raster_ranked"}, "ground":{"type":"raster_ranked"}
            }, "nodes":{
                "current":{"op":"current","inputs":{"resistance":"inputs.resistance","source":"inputs.source","ground":"inputs.ground"}},
                "rounded":{"op":"round","inputs":"nodes.current"}
            }, "output":"nodes.rounded"
        }).to_string()).unwrap();
    let source = [1.0, 0.0, 0.0];
    let ground = [0.0, 1.0, 1.0];
    let resistance = [1.0, 2.0, 4.0];
    let expected = CurrentSolver::new(&source, &ground, Default::default())
        .solve_current(&resistance, 1, 3)
        .unwrap();
    let mut results = Vec::new();
    graph
        .execute(
            HashMap::from([
                (
                    "resistance".into(),
                    PipelineData::raster(resistance.to_vec()),
                ),
                ("source".into(), PipelineData::raster(source.to_vec())),
                ("ground".into(), PipelineData::raster(ground.to_vec())),
            ]),
            grid(),
            options(3),
            None,
            |result| {
                results.push(result);
                Ok(())
            },
        )
        .unwrap();
    for result in results
        .iter()
        .filter(|result| result.layer == OutputLayer::Resistance && result.sample.is_some())
    {
        assert_eq!(
            result.outputs["data"].values().unwrap(),
            expected
                .iter()
                .map(|value| value.round())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn long_paths_execute_without_recursion() {
    let mut model = json!({"name":"deep", "inputs":{"data":{"type":"raster_ranked"}},
            "nodes":{}, "output":"nodes.n1999"});
    let mut input = "inputs.data".to_owned();
    for index in 0..2000 {
        let name = format!("n{index:04}");
        model["nodes"][&name] = json!({"op":"round", "inputs":input});
        input = format!("nodes.{name}");
    }
    let graph = PipelineGraph::from_json(&model.to_string()).unwrap();
    let results = run(
        &graph,
        ExecutionOptions {
            all_layers: false,
            ..options(2)
        },
        None,
    );
    let summary = results
        .iter()
        .find(|result| result.sample.is_none())
        .unwrap();
    assert_eq!(summary.outputs["mean"].values().unwrap(), &[1.0, 2.0, 4.0]);
    assert_eq!(summary.outputs["variance"].values().unwrap(), &[0.0; 3]);
}

#[test]
fn missing_cells_remain_missing_and_equal_draw_bounds_have_zero_variance() {
    let graph = model(json!({"sample":{"log_uniform":[2,2]}}));
    let mut results = Vec::new();
    graph
        .execute(
            HashMap::from([(
                "data".into(),
                PipelineData::raster(vec![1.0, f64::NAN, 4.0]),
            )]),
            grid(),
            options(3),
            None,
            |result| {
                results.push(result);
                Ok(())
            },
        )
        .unwrap();
    let summary = results
        .iter()
        .find(|result| result.layer == OutputLayer::Resistance && result.sample.is_none())
        .unwrap();
    assert!(summary.outputs["mean"].values().unwrap()[1].is_nan());
    assert!(summary.outputs["variance"].values().unwrap()[1].is_nan());
    assert_eq!(summary.outputs["variance"].values().unwrap()[0], 0.0);
}
