#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
export LC_ALL=C
cargo build --release --features bin --bin scenario-solve

config_dir=$(mktemp -d)
trap 'rm -rf "$config_dir"' EXIT
for trace in cs-julia-bench.out rust-lowmem-bench.out rust-mixed-precision-low-mem-bench.out; do
  : > "$trace"
done

for scenario in point_lights dsm linear_res; do
  output="scripts/scenarios/$scenario/output"
  mkdir -p "$output"
  config="$config_dir/$scenario.ini"
  sed -E \
    -e 's/^([[:space:]]*precision[[:space:]]*=).*/\1 double/' \
    -e "s|^([[:space:]]*output_file[[:space:]]*=).*|\1 $output/circuitscape.out|" \
    -e '/^[[:space:]]*residual_tolerance[[:space:]]*=/d' \
    "scripts/scenarios/$scenario/Circuitscape.ini" > "$config"
  printf 'residual_tolerance = 1e-4\n' >> "$config"

  for run in {1..5}; do
    echo "$scenario: run $run/5, Circuitscape f64"
    printf '\nscenario=%s run=%s\nprecision=f64 tol=1e-4\n' "$scenario" "$run" >> cs-julia-bench.out
    /usr/bin/time -v julia --startup-file=no -e \
      'using Circuitscape; @assert pkgversion(Circuitscape) == v"6.1.0"; Circuitscape.compute(ARGS[1])' \
      "$config" >> cs-julia-bench.out 2>&1

    for precision in f64 mixed; do
      if [[ "$precision" == mixed ]]; then
        trace=rust-mixed-precision-low-mem-bench.out
        suffix=_mixed
        tol=1e-3
      else
        trace=rust-lowmem-bench.out
        suffix=
        tol=1e-4
      fi
      echo "$scenario: run $run/5, Rust low-memory MG $precision"
      printf '\nscenario=%s run=%s\nprecision=%s tol=%s\n' "$scenario" "$run" "$precision" "$tol" >> "$trace"
      /usr/bin/time -v target/release/scenario-solve \
        --resistance "scripts/scenarios/$scenario/resistance.asc" \
        --source "scripts/scenarios/$scenario/source.asc" \
        --ground "scripts/scenarios/$scenario/ground.asc" \
        --precision "$precision" \
        --coarsenu 25 --l0nu 2 --l1nu 5 \
        --coarseomega 0.4 --l0omega 0.95 --l0postnu 40 \
        --tol "$tol" --max-iter 100000 \
        --output "$output/rust_current$suffix.asc" >> "$trace" 2>&1
    done
  done
  python3 scripts/convert_to_png.py "$output/circuitscape_curmap.asc"
  for suffix in '' _mixed; do
    python3 scripts/convert_to_png.py "$output/rust_current$suffix.asc"
    python3 scripts/compare_asc.py "$output/circuitscape_curmap.asc" "$output/rust_current$suffix.asc"
  done
done
