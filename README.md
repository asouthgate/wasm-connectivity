# lmConnect: Low-memory Biological Connectivity Analysis

This software provides an implementation of connectivity analysis suitable for use in a memory-constrained environment (such as a Wasm/browser or high-throughput applications), using a geometric multigrid conjugate gradient solver. This can enable certain kinds of 
interactive connectivity modelling for biological systems, such as ecological mapping. It can also be used to enable greater parallelism, such as in sampling algorithms.

## Quick-Start

To build and use the command-line application:

```sh
cargo build --release --features bin --bins
```

To run the `lmc-current` solver on example datasets, run

```sh
./target/release/lmc-current \
  --resistance example/public/geodata/base_resistance_500.asc \
  --source example/public/geodata/source_500.asc \
  --ground example/public/geodata/ground_500.asc \
  --precision mixed \
  --output current.asc
```

To run a resistance pipeline, see the [the Greater Horseshoe example](example/horseshoe-bat-model.json). 
We use a simple DAG system to define configurable process pipelines specified via JSON.
See [Resistance pipeline JSON specification](docs/pipeline-json-spec.md) for details.

```sh
./target/release/lmc-resistance \
  --model example/horseshoe-bat-model.json \
  --dtm example/data/resistance/dtm.tif \
  --dsm example/data/resistance/dsm.tif \
  --landcover example/data/resistance/lcm.tif \
  --buildings example/data/resistance/buildings.gpkg \
  --roads example/data/resistance/roads.geojson \
  --rivers example/data/resistance/empty.geojson \
  --generic example/data/resistance/empty.geojson \
  --lamps example/data/resistance/empty.geojson \
  --all-layers \
  --output-dir results/
```

See `lmc-resistance --help` for command-line flags.

To run the `lmc-current` solver on that resistance map in low-memory mixed-precision mode:

```sh
./target/release/lmc-current \
  --resistance results/total_resistance.tif \
  --source example/data/resistance/source.tif \
  --ground example/data/resistance/ground.tif \
  --precision mixed \
  --output current.tif
```

See `lmc-current --help` for command-line flags.

## Prerequisites

- CLI and library: [Rust](https://rustup.rs)
- CLI GIS input/output: GDAL development libraries and `pkg-config` (e.g. `sudo apt install libgdal-dev pkg-config` on Debian/Ubuntu).
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

- [Resistance pipeline JSON specification](docs/pipeline-json-spec.md)
- [Benchmarks](benchmarks/README.md)
- [JavaScript API](lib/README.md)
- [Example](example/README.md)
- [Paper](docs/paper/README.md)
- [Architecture](docs/architecture.md)
- [License](COPYING.md)

## Cargo features

The Rust crate here has a few feature flags. 

- `bin`: compiles the main CLI executables. 
- `developer`: enables the `lmc-profile` executable.
- `instrumentation-profile`: enables solver memory instrumentation in the library.
