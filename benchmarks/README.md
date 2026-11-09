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

To run the Circuitscape-to-rust comparison. Run:

```sh
bash benchmarks/scripts/run-cs-comparison.sh
python3 benchmarks/scripts/plot_cs_vs_rust_lowmem.py benchmarks/results/cs-julia-bench.out benchmarks/results/rust-lowmem-bench.out
```

## Browser

Some of the paper benchmarks are run directly from the browser. 
To build these, run:

```sh
make serve
```

Navigate to the relevant tabs, run the benchmarks, download the csv file, and then run:

```sh
python3 benchmarks/scripts/plot_benchmark.py benchmark.csv
```


## Memory profiling

To run the `prof-solve` executable with Valgrind, run: 

```sh
bash benchmarks/scripts/profile-mem.sh
```

## Instrumentation

Some instrumentation code has been developed. To make use of this for memory profiling, run:

```sh
mkdir -p benchmarks/results
cargo run --profile release-prof --features bin,instrumentation-profile --bin instrumentation-profile -- 500 --solver mg --ground shunt > benchmarks/results/profile.json
python3 benchmarks/scripts/plot_instrumentation_profile.py benchmarks/results/profile.json
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



## Sensitivity analysis

For the sensitivity analysis, three scripts are required.
The first generates current maps across light resistance parameters for the point-lights scenario. 
The second post-processes this, ready for plotting. 
The third plots the results. Step 2 and 3 are decoupled because iterating over image results and 
aggregating them can be slow. 

```sh
python3 benchmarks/scripts/sample_light_resistance.py --help
python3 benchmarks/scripts/postprocess_light_samples.py --help
python3 benchmarks/scripts/plot_light_samples.py --help
```
