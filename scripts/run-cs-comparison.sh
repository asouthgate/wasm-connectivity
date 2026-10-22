#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
export LC_ALL=C

cargo build --release --features bin --bin scenario-solve

config_dir=$(mktemp -d)
trap 'rm -rf "$config_dir"' EXIT

for trace in cs-julia-bench.out rust-lowmem-bench.out cs-julia-f32-bench.out rust-lowmem-f32-bench.out; do
  : > "$trace"
done

for scenario in point_lights dsm linear_res; do
  output="scripts/scenarios/$scenario/output"
  mkdir -p "$output"
  for precision in f64 f32; do
    if [[ "$precision" == f32 ]]; then
      cs_precision=single
      tol=1e-3
      suffix=_f32
      cs_trace=cs-julia-f32-bench.out
      rust_trace=rust-lowmem-f32-bench.out
    else
      cs_precision=double
      tol=1e-4
      suffix=
      cs_trace=cs-julia-bench.out
      rust_trace=rust-lowmem-bench.out
    fi
    config="$config_dir/$scenario-$precision.ini"
    sed -E \
      -e "s/^([[:space:]]*precision[[:space:]]*=).*/\1 $cs_precision/" \
      -e "s|^([[:space:]]*output_file[[:space:]]*=).*|\1 $output/circuitscape$suffix.out|" \
      -e '/^[[:space:]]*residual_tolerance[[:space:]]*=/d' \
      "scripts/scenarios/$scenario/Circuitscape.ini" > "$config"
    printf 'residual_tolerance = %s\n' "$tol" >> "$config"

    for run in {1..5}; do
      echo "$scenario: run $run/5, Circuitscape $precision"
      printf '\nscenario=%s run=%s\nprecision=%s tol=%s\n' "$scenario" "$run" "$precision" "$tol" >> "$cs_trace"
      /usr/bin/time -v julia --startup-file=no -e \
        'using Circuitscape; @assert pkgversion(Circuitscape) == v"6.1.0"; Circuitscape.compute(ARGS[1])' \
        "$config" >> "$cs_trace" 2>&1

      echo "$scenario: run $run/5, Rust low-memory MG $precision"
      printf '\nscenario=%s run=%s\nprecision=%s tol=%s\n' "$scenario" "$run" "$precision" "$tol" >> "$rust_trace"
      /usr/bin/time -v target/release/scenario-solve \
        --resistance "scripts/scenarios/$scenario/resistance.asc" \
        --source "scripts/scenarios/$scenario/source.asc" \
        --ground "scripts/scenarios/$scenario/ground.asc" \
        --precision "$precision" \
        --coarsenu 25 --l0nu 2 --l1nu 5 \
        --coarseomega 0.4 --l0omega 0.95 --l0postnu 40 \
        --tol "$tol" --max-iter 100000 \
        --output "$output/rust_current$suffix.asc" >> "$rust_trace" 2>&1
    done

    python3 scripts/convert_to_png.py "$output/circuitscape${suffix}_curmap.asc"
    python3 scripts/convert_to_png.py "$output/rust_current$suffix.asc"
    python3 scripts/compare_asc.py "$output/circuitscape${suffix}_curmap.asc" "$output/rust_current$suffix.asc"
  done
done

