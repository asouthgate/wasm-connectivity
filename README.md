# lmConnect: Low-memory Biological Connectivity Analysis

This software provides an implementation of connectivity analysis suitable for use in Wasm/browser
context, using a geometric multigrid conjugate gradient solver. This can enable certain kinds of 
interactive connectivity modelling for biological systems, such as ecological mapping. It can also be used to enable greater parallelism, such as in sampling algorithms.

## Quick-Start

To build and use the command-line application:

```sh
cargo build --release --features bin --bins
```

## Prerequisites

- CLI and library: [Rust](https://rustup.rs)
- Wasm integration: [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/)
- Browser example: [Node.js](https://nodejs.org)

## Build

This repository contains both a Rust library, CLI utilities, examples, and benchmarking and evaluation codes.

To build the entire project with `make`: 

```
make build
```

To build the CLI utiliies:

```
make cli
```

To also build the scenario/benchmark extras (the `lmc-current` binary and
the prepared scenario rasters), run:

```
make bench
```

## Example frontend

```
make serve
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
