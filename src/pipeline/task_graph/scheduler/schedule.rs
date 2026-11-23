use super::super::graph::{NodeOperation, NodeTaskList};
use std::collections::VecDeque;

enum SampleStep {
    Calculate(usize),
    UpdateStream(usize),
}

pub(super) struct Schedule {
    shared_steps: VecDeque<usize>,
    sample_steps: Vec<SampleStep>,
    sample_calculations: Vec<Vec<usize>>,
    final_steps: VecDeque<usize>,
}

fn streaming_destinations<'graph>(
    task_list: &'graph NodeTaskList<'_>,
    source_node: usize,
) -> impl Iterator<Item = usize> + 'graph {
    task_list.nodes[source_node]
        .output_edges
        .iter()
        .filter_map(|&edge| {
            let destination = task_list.edges[edge].destination_node;
            matches!(
                task_list.nodes[destination].operation,
                NodeOperation::Streaming { .. }
            )
            .then_some(destination)
        })
}

impl Schedule {
    pub(super) fn batches(self, threads: usize) -> impl Iterator<Item = VecDeque<usize>> {
        let Self {
            shared_steps,
            sample_steps,
            sample_calculations,
            final_steps,
        } = self;
        let sample_batches =
            (0..sample_calculations.len())
                .step_by(threads)
                .map(move |first_sample| {
                    let end = first_sample + threads.min(sample_calculations.len() - first_sample);
                    let mut steps = VecDeque::new();
                    for calculations in &sample_calculations[first_sample..end] {
                        steps.extend(sample_steps.iter().map(|step| match *step {
                            SampleStep::Calculate(position) => calculations[position],
                            SampleStep::UpdateStream(node_index) => node_index,
                        }));
                    }
                    steps
                });
        std::iter::once(shared_steps)
            .chain(sample_batches)
            .chain(std::iter::once(final_steps))
            .filter(|steps| !steps.is_empty())
    }

    pub(super) fn generate(task_list: &NodeTaskList<'_>, samples: usize) -> Self {
        let mut shared_steps = VecDeque::new();
        let mut sample_steps = Vec::new();
        let mut sample_calculations = vec![Vec::new(); samples];
        let mut final_steps = VecDeque::new();

        // The builder gives every sample the same calculations in topological order.
        for (node_index, node) in task_list.nodes.iter().enumerate() {
            let stream_updates = streaming_destinations(task_list, node_index);
            match node.operation {
                NodeOperation::Input => shared_steps.extend(stream_updates),
                NodeOperation::Calculate { .. } if node.is_shared => {
                    shared_steps.push_back(node_index);
                    shared_steps.extend(stream_updates);
                }
                NodeOperation::Calculate { .. } => {
                    let calculations = &mut sample_calculations[node.sample];
                    if node.sample == 0 {
                        sample_steps.push(SampleStep::Calculate(calculations.len()));
                        sample_steps.extend(stream_updates.map(SampleStep::UpdateStream));
                    }
                    calculations.push(node_index);
                }
                NodeOperation::Streaming { .. } => final_steps.extend(stream_updates),
            }
        }
        Self {
            shared_steps,
            sample_steps,
            sample_calculations,
            final_steps,
        }
    }
}
