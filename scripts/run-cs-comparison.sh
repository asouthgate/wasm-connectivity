#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
export LC_ALL=C

cargo build --release --features bin --bin scenario-solve

cs_trace=cs-julia-bench.out
rust_trace=rust-lowmem-bench.out
: > "$cs_trace"
: > "$rust_trace"

for scenario in point_lights dsm linear_res; do
  mkdir -p "scripts/scenarios/$scenario/output"
  for run in {1..5}; do
    echo "$scenario: run $run/5, Circuitscape"
    printf '\nscenario=%s run=%s\n' "$scenario" "$run" >> "$cs_trace"
    /usr/bin/time -v julia --startup-file=no -e \
      'using Circuitscape; @assert pkgversion(Circuitscape) == v"6.1.0"; Circuitscape.compute(ARGS[1])' \
      "scripts/scenarios/$scenario/Circuitscape.ini" >> "$cs_trace" 2>&1

    echo "$scenario: run $run/5, Rust low-memory MG"
    printf '\nscenario=%s run=%s\n' "$scenario" "$run" >> "$rust_trace"
    /usr/bin/time -v target/release/scenario-solve \
      --resistance "scripts/scenarios/$scenario/resistance.asc" \
      --source "scripts/scenarios/$scenario/source.asc" \
      --ground "scripts/scenarios/$scenario/ground.asc" \
      --tol 1e-4 --max-iter 100000 \
      --output "scripts/scenarios/$scenario/output/rust_current.asc" >> "$rust_trace" 2>&1
  done

  output="scripts/scenarios/$scenario/output"
  python3 scripts/convert_to_png.py "$output/circuitscape_curmap.asc"
  python3 scripts/convert_to_png.py "$output/rust_current.asc"
  python3 scripts/compare_asc.py "$output/circuitscape_curmap.asc" "$output/rust_current.asc"
done

