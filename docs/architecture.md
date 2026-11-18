# Overview

src/ contains source code for the rust library.
example/ contains source code for a react app using the rust library via Wasm. 

# Rust library

The Rust library allows resistance pipelines to be loaded and run. 
It also allows current maps to be solved using a custom MGCG solver.
As described in the README, resistance pipelines are defined with JSON models.

The top level src/ has the following subdirectories with separate concerns:
- bin/ - CLI codes
- browser/ - Wasm glue
- current/ - the main codes for solving linear current systems with low-memory 
- numeric/ - some helpers for numerics that are shared
- pipeline/ - the code responsible for pipeline DAGs, JSON specs, sampling, multithreaded excution
- raster/ - utilities for raster data, including rasterisation of vector data
- resistance/ - utilities for calculating resistance maps, including various operations such as raycasting
- roost/ - utilities for roost finding

Most of this is straightforward. The non-trivial parts are:

- pipeline/
  - model/ - contains code for defining the DAG models, it's a simple pipeline spec.
  - task_graph/ - builds a second graph for execution. This is more complex, as nodes that have sampling introduce complexity, especially when there is additional requirements to accumulate results into mean and variance maps
    - task_graph/execution.rs:execute shows the entrypoint into executing a graph. It builds a larger graph and distributes work in an efficient manner while minimising memory consumption, unlike simply running a pipeline multiple times in different processes, which would require duplication of input memory.
- current/ - the main low-memory MGCG linear solver code. Mathematically this is complex. See the paper.
  - current/stencil.rs - the main function entrypoints to the solvers
    - current/stencil.rs:solve_raster_sources_mg_stencil_with_options the main solver function