# lmConnect: Low-memory Biological Connectivity Analysis

This software provides an implementation of connectivity analysis suitable for use in a memory-constrained environment (such as a Wasm/browser or high-throughput applications), using a geometric multigrid conjugate gradient solver. This can enable certain kinds of 
interactive connectivity modelling for biological systems, such as ecological mapping. It can also be used to enable greater parallelism, such as in sampling algorithms.

## Quick-Start

To build and use the command-line application:

```sh
cargo build --release --features bin --bins
```

Use `lmc resistance` to generate resistance, `lmc current` to solve an existing
resistance raster, or `lmc full` to generate resistance and solve current in one run.
See `lmc --help` and `lmc <command> --help` for options.

To run the `lmc current` solver on example datasets, run

```sh
./target/release/lmc current \
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
./target/release/lmc resistance \
  --threads 4 \
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

See `lmc resistance --help` for command-line flags.

To run the `lmc current` solver on that resistance map in low-memory mixed-precision mode:

```sh
./target/release/lmc current \
  --resistance results/total_resistance.tif \
  --source example/data/resistance/source.tif \
  --ground example/data/resistance/ground.tif \
  --precision mixed \
  --output current.tif
```

See `lmc current --help` for command-line flags.

### Sampling resistance and current

Copy the example pipeline to sample road resistance with a narrow buffer:

```sh
cp example/horseshoe-bat-model.json sampled-model.json
```

In `sampled-model.json`, replace the entire `nodes.road_res` object with:

```json
{
  "input": "inputs.roads",
  "operations": [
    {
      "op": "distance",
      "parameters": {
        "units": "metres",
        "when_empty": "infinity"
      }
    },
    {
      "op": "power_response",
      "parameters": {
        "input_range": [0, 10],
        "base_range": [1, 0],
        "exponent": 1,
        "scale": { "sample": { "log_uniform": [10, 1000000] } },
        "offset": 0,
        "above": 0
      }
    }
  ]
}
```

Each sample draws a peak road resistance between 10 and 1,000,000, with equal
probability across powers of ten.

Run five samples with seed 42, calculating current from each resistance map and
saving every sample:

```sh
./target/release/lmc full \
  --threads 4 \
  --model sampled-model.json \
  --dtm example/data/resistance/dtm.tif \
  --dsm example/data/resistance/dsm.tif \
  --landcover example/data/resistance/lcm.tif \
  --buildings example/data/resistance/buildings.gpkg \
  --roads example/data/resistance/roads.geojson \
  --rivers example/data/resistance/empty.geojson \
  --generic example/data/resistance/empty.geojson \
  --lamps example/data/resistance/empty.geojson \
  --source example/data/resistance/source.tif \
  --ground example/data/resistance/ground.tif \
  --samples 5 --seed 42 --save-samples \
  --output-dir results-sampled/
```

See `results-sampled` for the results.

For the same five samples and summary maps without saving individual samples,
use the same seed and omit `--save-samples`:

```sh
./target/release/lmc full \
  --threads 4 \
  --model sampled-model.json \
  --dtm example/data/resistance/dtm.tif \
  --dsm example/data/resistance/dsm.tif \
  --landcover example/data/resistance/lcm.tif \
  --buildings example/data/resistance/buildings.gpkg \
  --roads example/data/resistance/roads.geojson \
  --rivers example/data/resistance/empty.geojson \
  --generic example/data/resistance/empty.geojson \
  --lamps example/data/resistance/empty.geojson \
  --source example/data/resistance/source.tif \
  --ground example/data/resistance/ground.tif \
  --samples 5 --seed 42 \
  --output-dir results-summary/
```

The second run avoids storing individual maps on disk, accumulating pipeline results during sampling.

## Prerequisites

- CLI and library: [Rust](https://rustup.rs)
- CLI GIS input/output: GDAL development libraries and `pkg-config` (e.g. `sudo apt install libgdal-dev pkg-config` on Debian/Ubuntu).
- Wasm integration: [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/) and the nightly Rust toolchain pinned in the Makefile, with `rust-src` and the `wasm32-unknown-unknown` target. Use `make build WASM_THREADS=0` for a single-threaded Wasm build.
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

To also build the scenario/benchmark extras (the `lmc` executable and
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
