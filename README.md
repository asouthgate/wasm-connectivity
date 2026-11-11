# lmConnect: Low-memory Biological Connectivity Analysis

This software provides an implementation of connectivity analysis suitable for use in a memory-constrained environment (such as a Wasm/browser or high-throughput applications), using a geometric multigrid conjugate gradient solver. This can enable certain kinds of 
interactive connectivity modelling for biological systems, such as ecological mapping. It can also be used to enable greater parallelism, such as in sampling algorithms.

## Quick-Start

To build and use the command-line application:

```sh
cargo build --release --features bin --bins
```

To compute resistance, start with [the horseshoe-bat model](example/horseshoe-bat-model.json)
and edit its `inputs` paths to point to your raster and vector data. Paths are
relative to the JSON model file. The model defines the operations for each layer
and how those layers combine, including interval mappings such as `"(1,3)": 4`.

```sh
./target/release/lmc-resistance \
  --model example/horseshoe-bat-model.json \
  --output-dir results/
```

This writes `results/total_res.tif`, `results/resistance.asc`, and
`results/grid_info.json`. All declared inputs must exist and raster inputs must
share the model grid's dimensions, alignment, and projected CRS in metres.

See `lmc-resistance --help` for command-line flags, including the legacy
`--config` mode.

To run the `lmc-current` solver on an example resistance map in low-memory mixed-precision mode:

```sh
./target/release/lmc-current \
  --resistance example/public/geodata/base_resistance_500.asc \
  --source example/public/geodata/source_500.asc \
  --ground example/public/geodata/ground_500.asc \
  --precision mixed \
  --output current.asc
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
