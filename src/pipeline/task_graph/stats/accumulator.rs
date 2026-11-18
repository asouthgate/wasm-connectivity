use crate::pipeline::{NodeOutputs, PipelineData, Result};
use std::collections::BTreeMap;

pub(in crate::pipeline::task_graph) struct StatisticsAccumulator {
    count: usize,
    mean: Vec<f64>,
    m2: Vec<f64>,
}
impl StatisticsAccumulator {
    pub(in crate::pipeline::task_graph) fn create() -> Box<dyn super::super::graph::StreamingNode> {
        Box::new(Self {
            count: 0,
            mean: Vec::new(),
            m2: Vec::new(),
        })
    }
    fn update(&mut self, values: &[f64]) -> Result<()> {
        if self.count == 0 {
            self.mean = vec![0.0; values.len()];
            self.m2 = vec![0.0; values.len()];
        }
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
}

impl super::super::graph::StreamingNode for StatisticsAccumulator {
    fn update(&mut self, inputs: &BTreeMap<String, Vec<PipelineData>>) -> Result<()> {
        let values = inputs
            .get("data")
            .and_then(|values| values.first())
            .ok_or("missing statistics input")?;
        self.update(values.values()?)
    }

    fn finish(mut self: Box<Self>) -> Result<NodeOutputs> {
        if self.count < 2 {
            return Err("sample variance requires at least two samples".into());
        }
        let divisor = (self.count - 1) as f64;
        for value in &mut self.m2 {
            *value /= divisor;
        }
        Ok(BTreeMap::from([
            ("mean".into(), PipelineData::raster(self.mean)),
            ("variance".into(), PipelineData::raster(self.m2)),
        ]))
    }
}
