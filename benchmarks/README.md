# Benchmarks

This repository contains code for benchmarking used for the repository paper and performance assessment.
For details on these benchmarks, the paper should be consulted. Detailed information is not duplicated
in this README.

## Dependencies

These loose benchmarking scripts require Julia, Python3, and a few dependencies, including Circuitscape.

```sh
python3 -m venv .venv
. .venv/bin/activate
python3 -m pip install numpy matplotlib rasterio scipy
julia --startup-file=no -e 'using Pkg; Pkg.add(Pkg.PackageSpec(name="Circuitscape", version="6.1.0"))'
```

## Scenarios

Scenarios are described in `docs/paper`. There are three scenarios derived from real data. 
Using these scenarios requires building. Run:

```sh
make bench
python3 benchmarks/scripts/scenario_metrics.py
```

## Comparison

To run the Circuitscape-to-rust benchmarking comparison. Run:

```sh
make bench
bash benchmarks/scripts/run-cs-comparison.sh
python3 benchmarks/scripts/plot_cs_vs_rust_lowmem.py \
  benchmarks/results/rust-mixed-precision-low-mem-bench.out \
  benchmarks/results/rust-lowmem-bench.out benchmarks/results/cs-julia-bench.out \
  docs/paper/figures/cs_vs_wasm.png
```

### Paper current-map comparison

To generate the single-image Circuitscape and Rust image comparison with raster errors in raw and rescaled units, run:

```sh
bash benchmarks/scripts/run-paper-current-comparison.sh
```

## Browser

Some of the paper benchmarks are run directly from the browser. 
To build these, run:

```sh
make serve
```

Navigate to the relevant tabs, run the benchmarks, download the csv file, and then run:

## Chudleigh benchmark and convergence figure

From the repository root, generate the three residual traces:

```sh
make bench-inputs
cargo build --release --features developer --bin lmc-profile
mkdir -p benchmarks/results
target/release/lmc-profile \
  --resistance benchmarks/results/chudleigh/resistance_2000.asc \
  --source example/public/geodata/source_2000.asc --ground example/public/geodata/ground_2000.asc \
  --ground-mode shunt --solver mg --precision f64 --residuals \
  > /dev/null 2> benchmarks/results/chudleigh-explicit.log
target/release/lmc-profile \
  --resistance benchmarks/results/chudleigh/resistance_2000.asc \
  --source example/public/geodata/source_2000.asc --ground example/public/geodata/ground_2000.asc \
  --ground-mode shunt --solver mg-lowmem --precision f64 --residuals \
  > /dev/null 2> benchmarks/results/chudleigh-lowmem.log
target/release/lmc-profile \
  --resistance benchmarks/results/chudleigh/resistance_2000.asc \
  --source example/public/geodata/source_2000.asc --ground example/public/geodata/ground_2000.asc \
  --ground-mode shunt --solver mg-lowmem --precision mixed --residuals \
  > /dev/null 2> benchmarks/results/chudleigh-mixed.log
```

Plot the benchmark CSV and saved traces together:

```sh
python3 benchmarks/scripts/plot_benchmark.py benchmarks/data/benchmarks.csv \
  benchmarks/results/chudleigh-explicit.log benchmarks/results/chudleigh-lowmem.log \
  benchmarks/results/chudleigh-mixed.log --output docs/paper/figures/benchmarking.png
```

## Instrumentation

Build the developer-only profiling executable with `--features developer`.
Pass aligned ASC or GeoTIFF files with `--resistance`, `--source`, and `--ground`. Add
`--timings` for timing tables or `--residuals` for residual traces on stderr.

Some instrumentation code has been developed. To make use of this for memory profiling, run:

```sh
make bench-inputs
mkdir -p benchmarks/results
cargo run --profile release-prof --features developer --bin lmc-profile -- \
  --resistance benchmarks/results/chudleigh/resistance_500.asc \
  --source example/public/geodata/source_500.asc --ground example/public/geodata/ground_500.asc \
  --solver mg --ground-mode shunt > benchmarks/results/profile.json
python3 benchmarks/scripts/plot_instrumentation_profile.py benchmarks/results/profile.json
```

Then run with `--solver mg-lowmem` for the `lowmem` version:

```sh
cargo run --profile release-prof --features developer \
  --bin lmc-profile -- \
  --resistance benchmarks/results/chudleigh/resistance_500.asc \
  --source example/public/geodata/source_500.asc --ground example/public/geodata/ground_500.asc \
  --solver mg-lowmem --ground-mode shunt > profile-lowmem.json
python3 benchmarks/scripts/plot_instrumentation_profile.py profile-lowmem.json
```

To visualise these results run:

```
python3 benchmarks/scripts/plot_instrumentation_profile.py profile-lowmem.json
```

## Sensitivity analysis

For the sensitivity analysis, three scripts are required.
The first generates current maps across light resistance parameters for the point-lights scenario using native sampling.
The second post-processes this, ready for plotting.
The third plots the results. Step 2 and 3 are decoupled because iterating over image results and
aggregating them can be slow.

```sh
make cli
/usr/bin/time -v bash benchmarks/scripts/run-sampling.sh tmp_sampling
python3 benchmarks/scripts/postprocess_light_samples.py tmp_sampling
python3 benchmarks/scripts/plot_light_samples.py \
  --cache tmp_sampling/plot_data.npz --output docs/paper/figures/current-standard-deviation.png
```
