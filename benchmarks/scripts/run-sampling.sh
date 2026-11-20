#!/usr/bin/env bash
set -eu

scenario_directory=benchmarks/scenarios/point_lights
inputs="$scenario_directory/sampling-inputs"
output=${1:?"Usage: bash run-sampling.sh OUTPUT_DIRECTORY"}
if [[ -e "$output" ]]; then
    printf 'Output already exists: %s\n' "$output" >&2
    exit 1
fi

echo "Running sampling benchmark with the following parameters:
  Scenario directory: $scenario_directory
  Inputs directory: $inputs
  Output directory: $output"

exec bin/lmc full --model "$scenario_directory/sampling-model.json" \
    --fixed "$inputs/fixed.tif" --light "$inputs/light.tif" \
    --source "$inputs/source.tif" --ground "$inputs/ground.tif" \
    --samples 100 --threads 16 --seed 0 --save-samples --output-dir "$output" -v
