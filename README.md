# wasm-connect

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

To also build the scenario/benchmark extras (the `scenario-solve` binary and
the prepared scenario rasters), run:

```
make bench
```

To build the optional profiling dependencies, see (or run) `benchmarks/scripts/profile-mem.sh`.

## Serve the example frontend

```
make serve
```

This frontend contains utilities for benchmarking the code in the browser.
To plot benchmarking results downloaded from the browser, run:

```sh
python3 benchmarks/scripts/plot_benchmark.py benchmark.csv
```

## Profiling

Additional command-line utilities can be used to profile the main algorithm.
The optional `instrumentation-profile` feature records analytical payload sizes
for specific solver buffers for optimisation purposes. 
Then use the plotter for visualisation.

For example, to run without the `lowmem` flag, run:


```sh
cargo run --profile release-prof --features bin,instrumentation-profile \
  --bin instrumentation-profile -- 500 --solver mg --ground shunt > profile.json
python3 benchmarks/scripts/plot_instrumentation_profile.py profile.json
```

Then run with `--solver mg-lowmem` for the `lowmem` version:

```sh
cargo run --profile release-prof --features bin,instrumentation-profile \
  --bin instrumentation-profile -- 500 --solver mg-lowmem --ground shunt > profile-lowmem.json
python3 benchmarks/scripts/plot_instrumentation_profile.py profile-lowmem.json
```

To visualise these results run:

```
python3 benchmarks/scripts/plot_instrumentation_profile.py profile-lowmem.json
```

## Rust

To build the Rust crate directly with Cargo:

```sh
cargo build
cargo test
cargo doc --no-deps --open
```

## Tests

To run the tests:

```sh
make test
```

## Documentation

- [Benchmarks](benchmarks/README.md)
- [JavaScript API](lib/README.md)
- [Example](example/README.md)
- [Paper](docs/paper/README.md)
- [Architecture](docs/architecture.md)
- [License](COPYING.md)

## Cargo features

The Rust crate here has a few feature flags. 

- `bin`: compiles the project CLI executables. 
- `instrumentation-profile`: compiles the instrumentation profiling script for development and benchmarking.
