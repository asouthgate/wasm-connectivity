Install extra dependencies:

```bash
julia --startup-file=no -e 'using Pkg; Pkg.add(Pkg.PackageSpec(name="Circuitscape", version="6.1.0"))'
python3 -m pip install numpy matplotlib rasterio
```

Build the required example files:

```bash
make build
```

Report scenario metrics:

```bash
python3 scripts/scenario_metrics.py
```

Run the benchmarking comparison script:

```bash
bash scripts/run-cs-comparison.sh
```

Generate the plots:

```bash
python3 scripts/plot_cs_vs_rust_lowmem.py cs-julia-bench.out rust-lowmem-bench.out
```
