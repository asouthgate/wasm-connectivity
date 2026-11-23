use super::{NodeExecutionState, NodeOperation, NodeTaskList, TaskNode};

#[derive(Default)]
pub(super) struct ExecutionTrace {
    enabled: bool,
    sources: usize,
    intermediates: usize,
}

fn sample_label(node: &TaskNode<'_>) -> String {
    if node.is_shared {
        "shared".into()
    } else if matches!(node.operation, NodeOperation::Streaming { .. }) {
        "summary".into()
    } else {
        format!("sample {}", node.sample)
    }
}

impl ExecutionTrace {
    pub(super) fn record_outputs(&mut self, node: &TaskNode<'_>, count: usize) {
        if self.enabled {
            if matches!(node.operation, NodeOperation::Input) {
                self.sources += count;
            } else {
                self.intermediates += count;
            }
        }
    }

    pub(super) fn release(&mut self, node: &TaskNode<'_>, port: &str) {
        if !self.enabled {
            return;
        }
        let kind = if matches!(node.operation, NodeOperation::Input) {
            self.sources -= 1;
            "source"
        } else {
            self.intermediates -= 1;
            "intermediate"
        };
        eprintln!(
            "Released {kind} {}.{port} [{}]; sources in flight: {}, intermediates in flight: {}",
            node.name,
            sample_label(node),
            self.sources,
            self.intermediates
        );
    }

    pub(super) fn finish_job(
        &mut self,
        task_list: &NodeTaskList<'_>,
        node_index: usize,
        input_edges: &[usize],
        output_count: usize,
    ) {
        self.record_outputs(&task_list.nodes[node_index], output_count);
        self.job_event("Finished", task_list, node_index, input_edges);
    }

    pub(super) fn start_job(
        &self,
        task_list: &NodeTaskList<'_>,
        node_index: usize,
        input_edges: &[usize],
    ) {
        self.job_event("Running", task_list, node_index, input_edges);
    }

    fn job_event(
        &self,
        event: &str,
        task_list: &NodeTaskList<'_>,
        node_index: usize,
        input_edges: &[usize],
    ) {
        if !self.enabled {
            return;
        }
        let node = &task_list.nodes[node_index];
        let job = if matches!(node.operation, NodeOperation::Streaming { .. }) {
            let edge = &task_list.edges[input_edges[0]];
            let source = &task_list.nodes[edge.source_node];
            format!(
                "{} for {}.{} [{}]",
                node.name,
                source.name,
                edge.output_port,
                sample_label(source)
            )
        } else {
            format!("{} [{}]", node.name, sample_label(node))
        };
        eprintln!(
            "{event} {job}; sources in flight: {}, intermediates in flight: {}",
            self.sources, self.intermediates
        );
    }

    pub(super) fn new(
        enabled: bool,
        task_list: &NodeTaskList<'_>,
        states: &[NodeExecutionState],
    ) -> Self {
        let mut trace = Self {
            enabled,
            ..Self::default()
        };
        if enabled {
            for (node, state) in task_list.nodes.iter().zip(states) {
                trace.record_outputs(node, state.outputs.len());
            }
        }
        trace
    }
}
