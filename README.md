This software provides an implementation of connectivity analysis suitable for use in Wasm/browser
context, using a geometric multigrid conjugate gradient solver. This can enable certain kinds of 
interactive connectivity modelling for biological systems, such as ecological mapping.

## Prerequisites

- [Rust](https://rustup.rs)
- [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/)
- [Node.js](https://nodejs.org)

## Build the project

```
make build
```

To build the optional profiling dependencies, see (or run) `scripts/profile-mem.sh`.

## Serve the example frontend

```
make serve
```

This frontend contains utilities for benchmarking the code in the browser.

## Profiling

There is additional code for profiling data structures at runtime.

The optional `instrumentation-profile` feature records analytical payload sizes
for specific solver buffers for optimisation purposes. 
Then use the plotter for visualisation.


```sh
cargo run --profile release-prof --features bin,instrumentation-profile \
  --bin instrumentation-profile -- 500 --solver mg --ground shunt > profile.json
python3 scripts/plot_instrumentation_profile.py profile.json
```

```sh
cargo run --profile release-prof --features bin,instrumentation-profile \
  --bin instrumentation-profile -- 500 --solver mg-lowmem --ground shunt > profile-lowmem.json
python3 scripts/plot_instrumentation_profile.py profile-lowmem.json
```

To plot benchmarking results:

```sh
python3 scripts/plot_benchmark.py benchmark.csv
```
