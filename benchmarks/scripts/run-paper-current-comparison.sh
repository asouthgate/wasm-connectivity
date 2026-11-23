#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
cargo build --release --features bin --bin lmc

work_dir=$(mktemp -d)
trap 'rm -rf "$work_dir"' EXIT
sed -e "s|^output_file = .*|output_file = $work_dir/results.out|" \
  -e 's/^write_as_tif = .*/write_as_tif = False/' tests/cs.ini > "$work_dir/cs.ini"
julia --startup-file=no -e 'using Circuitscape; Circuitscape.compute(ARGS[1])' "$work_dir/cs.ini"

target/release/lmc current \
  --resistance tests/output/resistance_with_building.asc \
  --source example/public/geodata/source_500.asc \
  --ground example/public/geodata/ground_500.asc \
  --precision f64 --tol 1e-6 --max-iter 100000 \
  --output "$work_dir/rust_current.asc"

mkdir -p docs/figures/images
python3 benchmarks/scripts/tif2png.py \
  "$work_dir/results_curmap.asc" "$work_dir/rust_current.asc" docs/figures/images
python3 benchmarks/scripts/compare_rasters.py \
  "$work_dir/results_curmap.asc" "$work_dir/rust_current.asc"
