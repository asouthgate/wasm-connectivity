// This contains code for the scheduler that executes a pipeline DAG with sampling.
// We first build a bigger graph: nodes whose results stay the same across samples
// appear once, and nodes whose results change appear once for each sample.
// We then connect those nodes and sort them by dependency.
// For this bigger graph, we refer to TaskNode and NodeEdge, and the graph is a NodeTaskList.
//
// Threads execute nodes when their input data is ready. Connections keep data
// available until every node that reads it has finished, after which it can be freed.
// Requested results are saved when the operation producing them finishes.
//
// There are three types of nodes: input nodes, calculation nodes, and streaming nodes.
// Input nodes hold supplied data. Calculation nodes run an operation once.
// Streaming nodes retain their state and process one available input at a time,
// returning the worker thread after each update. Statistics use streaming nodes
// to update means and variances without retaining every sample map.

mod builder;
mod current;
mod execution;
mod graph;
mod operations;
mod results;
mod sampling;
pub(super) mod scheduler;
mod stats;

pub(crate) use execution::execute;

#[cfg(test)]
mod tests;
