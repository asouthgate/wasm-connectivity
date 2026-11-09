# Benchmarks

-description here-

## Dependencies

-description here-

```sh
python3 -m venv .venv
. .venv/bin/activate
python3 -m pip install numpy matplotlib rasterio scipy
julia --startup-file=no -e 'using Pkg; Pkg.add(Pkg.PackageSpec(name="Circuitscape", version="6.1.0"))'
```

## Scenarios

-description here-

```sh
make bench
python3 benchmarks/scripts/scenario_metrics.py
```

## Comparison

-description here-

```sh
bash benchmarks/scripts/run-cs-comparison.sh
python3 benchmarks/scripts/plot_cs_vs_rust_lowmem.py benchmarks/results/cs-julia-bench.out benchmarks/results/rust-lowmem-bench.out
```

## Browser

-description here-

```sh
make serve
python3 benchmarks/scripts/plot_benchmark.py benchmark.csv
```

## Memory profiling

-description here-

```sh
bash benchmarks/scripts/profile-mem.sh
```

## Instrumentation

-description here-

```sh
mkdir -p benchmarks/results
cargo run --profile release-prof --features bin,instrumentation-profile --bin instrumentation-profile -- 500 --solver mg --ground shunt > benchmarks/results/profile.json
python3 benchmarks/scripts/plot_instrumentation_profile.py benchmarks/results/profile.json
```

## Sensitivity analysis

-description here-

```sh
python3 benchmarks/scripts/sample_light_resistance.py --help
python3 benchmarks/scripts/postprocess_light_samples.py --help
python3 benchmarks/scripts/plot_light_samples.py --help
```

## Inputs, outputs, interpretation, and reproducibility

-description here-
