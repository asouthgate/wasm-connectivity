use super::{NodeOutputs, Operation, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Draw {
    Values(Vec<f64>),
    Uniform([f64; 2]),
    LogUniform([f64; 2]),
}

#[derive(Clone, Debug)]
pub struct SampledParameters {
    template: Value,
    draws: Vec<(String, Draw)>,
}
impl SampledParameters {
    pub(super) fn extract(parameters: &mut Value) -> Result<Option<Self>> {
        let mut draws = Vec::new();
        fn visit(value: &mut Value, path: &str, draws: &mut Vec<(String, Draw)>) -> Result<()> {
            match value {
                Value::Object(object) if object.contains_key("sample") => {
                    if object.len() != 1 {
                        return Err(format!("{path}: sample must be the only field"));
                    }
                    let draw: Draw = serde_json::from_value(object["sample"].clone())
                        .map_err(|e| e.to_string())?;
                    let representative = match &draw {
                        Draw::Values(values) => {
                            if values.is_empty() || values.iter().any(|v| !v.is_finite()) {
                                return Err(format!(
                                    "{path}: sampled values must be nonempty and finite"
                                ));
                            }
                            values[0]
                        }
                        Draw::Uniform([lo, hi]) | Draw::LogUniform([lo, hi]) => {
                            if !lo.is_finite()
                                || !hi.is_finite()
                                || lo > hi
                                || matches!(draw, Draw::LogUniform(_)) && *lo <= 0.0
                            {
                                return Err(format!("{path}: invalid sampling bounds"));
                            }
                            *lo
                        }
                    };
                    draws.push((path.into(), draw));
                    *value = Value::from(representative);
                }
                Value::Object(object) => {
                    for (key, child) in object {
                        let key = key.replace('~', "~0").replace('/', "~1");
                        visit(child, &format!("{path}/{key}"), draws)?;
                    }
                }
                Value::Array(array) => {
                    for (i, child) in array.iter_mut().enumerate() {
                        visit(child, &format!("{path}/{i}"), draws)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        visit(parameters, "", &mut draws)?;
        Ok((!draws.is_empty()).then(|| Self {
            template: parameters.clone(),
            draws,
        }))
    }
    pub(super) fn validate_count(&self, count: usize) -> Result<()> {
        for (path, draw) in &self.draws {
            if let Draw::Values(values) = draw {
                if values.len() != count {
                    return Err(format!(
                        "{path}: expected {count} sample values, got {}",
                        values.len()
                    ));
                }
            }
        }
        Ok(())
    }
    pub(super) fn operation(&self, node: &str, index: usize, seed: u64) -> Result<Operation> {
        let mut parameters = self.template.clone();
        for (path, draw) in &self.draws {
            let value = match draw {
                Draw::Values(values) => values[index],
                Draw::Uniform([lo, hi]) | Draw::LogUniform([lo, hi]) => {
                    // SplitMix64's mixing function, with a stable key for each parameter
                    // and sample. Evaluation order cannot change the random draws.
                    fn mix(mut x: u64) -> u64 {
                        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
                        x ^ (x >> 31)
                    }
                    let hash = node
                        .bytes()
                        .chain([0])
                        .chain(path.bytes())
                        .fold(0xcbf29ce484222325u64, |hash, byte| {
                            (hash ^ byte as u64).wrapping_mul(0x100000001b3)
                        });
                    let bits = mix(seed
                        .wrapping_add(hash)
                        .wrapping_add((index as u64).wrapping_mul(0x9e3779b97f4a7c15)));
                    let u = (bits >> 11) as f64 / 9007199254740992.0;
                    if lo == hi {
                        *lo
                    } else if matches!(draw, Draw::LogUniform(_)) {
                        ((1.0 - u) * lo.ln() + u * hi.ln()).exp()
                    } else {
                        (1.0 - u) * lo + u * hi
                    }
                }
            };
            *parameters
                .pointer_mut(path)
                .ok_or_else(|| format!("unknown sampled parameter: {path}"))? = Value::from(value);
        }
        let operation: Operation = serde_json::from_value(parameters).map_err(|e| e.to_string())?;
        operation.validate_parameters()?;
        Ok(operation)
    }
}

pub struct SampleResult {
    pub index: usize,
    pub resistance: Arc<Vec<f64>>,
    pub current: Option<Vec<f64>>,
    pub layers: BTreeMap<String, NodeOutputs>,
}

pub struct MapStatistics {
    pub mean: Vec<f64>,
    pub variance: Vec<f64>,
}

pub struct SamplingResult {
    pub samples: usize,
    pub resistance: MapStatistics,
    pub current: Option<MapStatistics>,
    pub layers: BTreeMap<String, BTreeMap<String, MapStatistics>>,
}

struct Accumulator {
    count: usize,
    mean: Vec<f64>,
    m2: Vec<f64>,
}
impl Accumulator {
    fn new(size: usize) -> Self {
        Self {
            count: 0,
            mean: vec![0.0; size],
            m2: vec![0.0; size],
        }
    }
    fn update(&mut self, values: &[f64]) -> Result<()> {
        if values.len() != self.mean.len() {
            return Err("sample dimensions changed".into());
        }
        if values.iter().any(|v| v.is_infinite()) {
            return Err("infinite sample output".into());
        }
        self.count += 1;
        for ((mean, m2), &value) in self.mean.iter_mut().zip(&mut self.m2).zip(values) {
            let delta = value - *mean;
            *mean += delta / self.count as f64;
            *m2 += delta * (value - *mean);
            if mean.is_infinite() || m2.is_infinite() {
                return Err("sample statistics overflow".into());
            }
        }
        Ok(())
    }
    fn finish(mut self) -> MapStatistics {
        let divisor = (self.count - 1) as f64;
        for value in &mut self.m2 {
            *value /= divisor;
        }
        MapStatistics {
            mean: self.mean,
            variance: self.m2,
        }
    }
}

pub(super) struct ExecutionStatistics {
    resistance: Accumulator,
    current: Option<Accumulator>,
    layers: BTreeMap<String, BTreeMap<String, Accumulator>>,
}

impl ExecutionStatistics {
    pub(super) fn new(size: usize, current: bool) -> Self {
        Self {
            resistance: Accumulator::new(size),
            current: current.then(|| Accumulator::new(size)),
            layers: BTreeMap::new(),
        }
    }

    pub(super) fn update(&mut self, sample: &SampleResult) -> Result<()> {
        self.resistance.update(&sample.resistance)?;
        if let (Some(stats), Some(values)) = (&mut self.current, &sample.current) {
            stats.update(values)?;
        }
        let size = self.resistance.mean.len();
        for (name, outputs) in &sample.layers {
            for (port, field) in outputs {
                if let super::PipelineData::Raster(values) = field {
                    self.layers
                        .entry(name.clone())
                        .or_default()
                        .entry(port.clone())
                        .or_insert_with(|| Accumulator::new(size))
                        .update(values)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn finish(self) -> SamplingResult {
        SamplingResult {
            samples: self.resistance.count,
            resistance: self.resistance.finish(),
            current: self.current.map(Accumulator::finish),
            layers: self
                .layers
                .into_iter()
                .map(|(name, outputs)| {
                    (
                        name,
                        outputs
                            .into_iter()
                            .map(|(port, stats)| (port, stats.finish()))
                            .collect(),
                    )
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::execute_output;
    use super::super::{ExecutionOptions, Grid, PipelineData, PipelineGraph};
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn model() -> PipelineGraph {
        PipelineGraph::from_json(
            &json!({
                "name":"samples", "inputs":{"data":{"type":"raster_ranked"}},
                "nodes":{
                    "z_fixed":{"op":"round","inputs":"inputs.data"},
                    "b_sample":{"op":"power_response","inputs":"nodes.z_fixed","parameters":{
                        "input_range":[0,1],"base_range":[0,1],"exponent":1,
                        "scale":{"sample":{"values":[1,2,3]}},
                        "offset":{"sample":{"values":[0,1,2]}}
                    }},
                    "c_square":{"op":"power_response","inputs":"nodes.b_sample","parameters":{
                        "input_range":[0,5],"base_range":[0,5],"exponent":2,"scale":1,"offset":0
                    }},
                    "a_final":{"op":"power_response","inputs":"nodes.c_square","parameters":{
                        "input_range":[0,25],"base_range":[0,25],"exponent":1,"scale":1,"offset":10
                    }}
                },"output":"nodes.a_final"
            })
            .to_string(),
        )
        .unwrap()
    }
    fn grid() -> Grid {
        Grid {
            nrows: 1,
            ncols: 2,
            pixel_width: 1.0,
        }
    }
    fn inputs() -> HashMap<String, PipelineData> {
        HashMap::from([("data".into(), PipelineData::raster(vec![1.0, 0.0]))])
    }
    fn options(all_layers: bool) -> ExecutionOptions {
        ExecutionOptions {
            samples: 3,
            seed: 42,
            all_layers,
            statistics: true,
        }
    }
    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-10, "{a} != {b}");
    }

    #[test]
    fn one_draw_uses_the_shared_executor_without_statistics() {
        let pipeline_json = json!({"name":"single", "inputs":{"data":{"type":"raster_ranked"}},
            "nodes":{"result":{"op":"power_response","inputs":"inputs.data","parameters":{
                "input_range":[0,1],"base_range":[0,1],"exponent":1,
                "scale":{"sample":{"values":[3]}},"offset":0
            }}},"output":"nodes.result"})
        .to_string();
        let model = PipelineGraph::from_json(&pipeline_json).unwrap();
        let mut seen = 0;
        let statistics = super::super::execute(
            &pipeline_json,
            inputs(),
            grid(),
            ExecutionOptions {
                samples: 1,
                seed: 0,
                all_layers: true,
                statistics: false,
            },
            None,
            |sample| {
                assert_eq!(*sample.resistance, [3.0, 0.0]);
                assert_eq!(
                    sample.layers["result"]["data"].values().unwrap(),
                    [3.0, 0.0]
                );
                seen += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(seen, 1);
        assert!(statistics.is_none());
        assert_eq!(
            execute_output(&model, inputs(), grid()).unwrap(),
            [3.0, 0.0]
        );
    }

    #[test]
    fn ordinary_run_preserves_infinite_outputs_without_statistics() {
        let model = PipelineGraph::from_json(
            &json!({"name":"distance", "inputs":{"data":{"type":"raster_ranked"}},
            "nodes":{"result":{"op":"distance","inputs":"inputs.data","parameters":{
                "units":"metres","when_empty":"infinity"
            }}},"output":"nodes.result"})
            .to_string(),
        )
        .unwrap();
        let graph = model.clone();
        for all_layers in [false, true] {
            let mut seen = 0;
            let statistics = graph
                .execute(
                    HashMap::from([("data".into(), PipelineData::raster(vec![0.0, 0.0]))]),
                    grid(),
                    ExecutionOptions {
                        samples: 1,
                        seed: 0,
                        all_layers,
                        statistics: false,
                    },
                    None,
                    |sample| {
                        assert_eq!(*sample.resistance, [f64::INFINITY, f64::INFINITY]);
                        if all_layers {
                            assert_eq!(
                                sample.layers["result"]["data"].values().unwrap(),
                                sample.resistance.as_slice()
                            );
                        } else {
                            assert!(sample.layers.is_empty());
                        }
                        seen += 1;
                        Ok(())
                    },
                )
                .unwrap();
            assert_eq!(seen, 1);
            assert!(statistics.is_none());
        }
    }

    #[test]
    fn fixed_results_are_shared_and_all_descendants_recomputed_then_released() {
        let model = model();
        let graph = model.clone();
        assert_eq!(graph.order, ["z_fixed", "b_sample", "c_square", "a_final"]);
        let mut fixed = None;
        let mut previous = std::sync::Weak::new();
        let mut seen = 0;
        let result = graph
            .execute(inputs(), grid(), options(true), None, |sample| {
                assert!(
                    previous.upgrade().is_none(),
                    "previous sample is still retained"
                );
                let PipelineData::Raster(array) = &sample.layers["z_fixed"]["data"] else {
                    panic!()
                };
                if let Some(fixed) = &fixed {
                    assert!(Arc::ptr_eq(fixed, array));
                } else {
                    fixed = Some(array.clone());
                }
                close(sample.resistance[0], [11.0, 19.0, 35.0][sample.index]);
                close(
                    sample.layers["c_square"]["data"].values().unwrap()[0],
                    [1.0, 9.0, 25.0][sample.index],
                );
                previous = Arc::downgrade(&sample.resistance);
                seen += 1;
                Ok(())
            })
            .map(|statistics| statistics.expect("statistics were requested"))
            .unwrap();
        assert_eq!(seen, 3);
        assert!(previous.upgrade().is_none());
        close(result.resistance.mean[0], 65.0 / 3.0);
        close(result.resistance.variance[0], 448.0 / 3.0);
        assert_eq!(result.layers["z_fixed"]["data"].variance, [0.0, 0.0]);
        assert!(execute_output(&model, inputs(), grid())
            .unwrap_err()
            .contains("expected 1 sample values"));
    }

    #[test]
    fn current_is_solved_per_sample_before_statistics_are_accumulated() {
        let model = PipelineGraph::from_json(&json!({
            "name":"current", "inputs":{"data":{"type":"raster_ranked"}},
            "nodes":{"resistance":{"op":"power_response","inputs":"inputs.data","parameters":{
                "input_range":[1,4],"base_range":[1,4],"exponent":{"sample":{"values":[1,2,3]}},"scale":1,"offset":0
            }}},"output":"nodes.resistance"
        }).to_string()).unwrap();
        let graph = model.clone();
        let grid = Grid {
            nrows: 1,
            ncols: 3,
            pixel_width: 1.0,
        };
        let source = [1.0, 0.0, 0.0];
        let ground = [0.0, 1.0, 1.0];
        let current_solver = crate::pipeline::CurrentSolver::new(&source, &ground);
        let mut maps = Vec::new();
        let result = graph
            .execute(
                HashMap::from([("data".into(), PipelineData::raster(vec![1.0, 2.0, 4.0]))]),
                grid,
                options(false),
                Some(&current_solver),
                |sample| {
                    let expected = current_solver.solve_current(&sample.resistance, 1, 3)?;
                    assert_eq!(sample.current.as_ref().unwrap(), &expected);
                    maps.push(expected);
                    Ok(())
                },
            )
            .map(|statistics| statistics.expect("statistics were requested"))
            .unwrap();
        let stats = result.current.unwrap();
        for i in 0..3 {
            let mean = maps.iter().map(|map| map[i]).sum::<f64>() / 3.0;
            let variance = maps.iter().map(|map| (map[i] - mean).powi(2)).sum::<f64>() / 2.0;
            close(stats.mean[i], mean);
            close(stats.variance[i], variance);
        }
        let from_mean = current_solver
            .solve_current(&result.resistance.mean, 1, 3)
            .unwrap();
        assert!(from_mean
            .iter()
            .zip(&stats.mean)
            .any(|(a, b)| (a - b).abs() > 1e-4));
    }

    #[test]
    fn seeded_draws_are_repeatable_and_missing_cells_remain_missing() {
        for distribution in [json!({"uniform":[1,10]}), json!({"log_uniform":[1,10]})] {
            let model=PipelineGraph::from_json(&json!({"name":"random","inputs":{"data":{"type":"raster_ranked"}},
                "nodes":{"res":{"op":"power_response","inputs":"inputs.data","parameters":{
                    "input_range":[0,1],"base_range":[0,1],"exponent":1,"scale":{"sample":distribution},"offset":0
                }}},"output":"nodes.res"}).to_string()).unwrap();
            let graph = model.clone();
            let run = |seed| {
                graph
                    .execute(
                        HashMap::from([("data".into(), PipelineData::raster(vec![1.0, f64::NAN]))]),
                        grid(),
                        ExecutionOptions {
                            seed,
                            ..options(false)
                        },
                        None,
                        |sample| {
                            assert!(sample.resistance[0] >= 1.0 && sample.resistance[0] <= 10.0);
                            Ok(())
                        },
                    )
                    .map(|statistics| statistics.expect("statistics were requested"))
                    .unwrap()
            };
            let a = run(42);
            let b = run(42);
            let c = run(43);
            assert_eq!(a.resistance.mean[0], b.resistance.mean[0]);
            assert_eq!(a.resistance.variance[0], b.resistance.variance[0]);
            assert_ne!(a.resistance.mean[0], c.resistance.mean[0]);
            assert!(a.resistance.mean[1].is_nan() && a.resistance.variance[1].is_nan());
        }
    }

    #[test]
    fn equal_distribution_bounds_produce_the_exact_constant_and_zero_variance() {
        for distribution in [json!({"uniform":[3,3]}), json!({"log_uniform":[3,3]})] {
            let model = PipelineGraph::from_json(&json!({"name":"constant", "inputs":{"data":{"type":"raster_ranked"}},
                "nodes":{"r":{"op":"power_response","inputs":"inputs.data","parameters":{
                    "input_range":[0,1],"base_range":[0,1],"exponent":1,"scale":{"sample":distribution},"offset":0
                }}},"output":"nodes.r"}).to_string()).unwrap();
            let result = model
                .clone()
                .execute(inputs(), grid(), options(false), None, |_| Ok(()))
                .map(|statistics| statistics.expect("statistics were requested"))
                .unwrap();
            assert_eq!(result.resistance.mean, [3.0, 0.0]);
            assert_eq!(result.resistance.variance, [0.0, 0.0]);
        }
    }

    #[test]
    fn separate_sampled_branches_combine_by_sample_index() {
        let response = |values| {
            json!({"op":"power_response","inputs":"inputs.data","parameters":{
                "input_range":[0,1],"base_range":[0,1],"exponent":1,"scale":{"sample":{"values":values}},"offset":0
            }})
        };
        let model = PipelineGraph::from_json(
            &json!({"name":"branches","inputs":{"data":{"type":"raster_ranked"}},
            "nodes":{"a":response(vec![1,2,3]),"b":response(vec![10,20,40]),
                "total":{"op":"sum","inputs":["nodes.a","nodes.b"]}},"output":"nodes.total"})
            .to_string(),
        )
        .unwrap();
        let mut seen = 0;
        let result = model
            .clone()
            .execute(inputs(), grid(), options(false), None, |sample| {
                assert_eq!(sample.resistance[0], [11.0, 22.0, 43.0][sample.index]);
                seen += 1;
                Ok(())
            })
            .map(|statistics| statistics.expect("statistics were requested"))
            .unwrap();
        assert_eq!(seen, 3);
        close(result.resistance.mean[0], 76.0 / 3.0);
        close(result.resistance.variance[0], 793.0 / 3.0);
    }

    #[test]
    fn invalid_samples_are_errors_and_do_not_fall_back_to_fixed_parameters() {
        let model = model();
        let graph = model.clone();
        assert!(graph
            .execute(
                inputs(),
                grid(),
                ExecutionOptions {
                    samples: 2,
                    ..options(false)
                },
                None,
                |_| Ok(())
            )
            .map(|statistics| statistics.expect("statistics were requested"))
            .err()
            .unwrap()
            .contains("sample values"));
        assert!(graph
            .execute(
                inputs(),
                grid(),
                ExecutionOptions {
                    samples: 1,
                    ..options(false)
                },
                None,
                |_| Ok(())
            )
            .map(|statistics| statistics.expect("statistics were requested"))
            .err()
            .unwrap()
            .contains("at least two"));
        let model=PipelineGraph::from_json(&json!({"name":"invalid","inputs":{"data":{"type":"raster_ranked"}},
            "nodes":{"r":{"op":"power_response","inputs":"inputs.data","parameters":{
                "input_range":[0,1],"base_range":[0,1],"exponent":{"sample":{"values":[1,-1,2]}},"scale":1,"offset":0
            }}},"output":"nodes.r"}).to_string()).unwrap();
        assert!(model
            .clone()
            .execute(inputs(), grid(), options(false), None, |_| Ok(()))
            .map(|statistics| statistics.expect("statistics were requested"))
            .err()
            .unwrap()
            .contains("sample 1, node r"));
    }

    #[test]
    fn long_sample_paths_execute_without_recursion() {
        let mut value=serde_json::to_value(json!({"name":"deep","inputs":{"data":{"type":"raster_ranked"}},
            "nodes":{"n0000":{"op":"power_response","inputs":"inputs.data","parameters":{
                "input_range":[0,1],"base_range":[0,1],"exponent":1,"scale":{"sample":{"values":[1,2,3]}},"offset":0
            }}},"output":"nodes.n1999"})).unwrap();
        for i in 1..2000 {
            value["nodes"][format!("n{i:04}")] =
                json!({"op":"round","inputs":format!("nodes.n{:04}",i-1)});
        }
        let model = PipelineGraph::from_json(&value.to_string()).unwrap();
        let result = model
            .clone()
            .execute(inputs(), grid(), options(false), None, |_| Ok(()))
            .map(|statistics| statistics.expect("statistics were requested"))
            .unwrap();
        assert_eq!(result.resistance.mean, [2.0, 0.0]);
        assert_eq!(result.resistance.variance, [1.0, 0.0]);
    }
}
