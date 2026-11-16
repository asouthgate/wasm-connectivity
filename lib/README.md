# JavaScript API

This directory contains code for the JavaScript API for the
scientific `lmConnect` code.

## Build

See the repository [build instructions](../README.md#build).

## Pipeline usage

Pass your own model JSON, named input arrays, and grid to
`runResistancePipelineBrowserAsync(modelJson, inputs, grid)`. The library executes
the operations defined by your model and returns `output` and named `layers`.

- `modelJson` is a JSON string containing your model definition.
- `inputs` supplies the data for each input named in your model. For example,
  `{ habitat: habitatData }` supplies `habitatData` for the model input called
  `"habitat"`. The data must be a `Float32Array`, `Float64Array`, or `number[]`.
- `grid` is `{ nrows: number, ncols: number, cellsize: number }`: positive integer
  row/column counts and a positive cell size in metres.

Raster inputs contain `nrows * ncols` values in row-major order, starting at the
northwest cell. Supply aligned arrays: `vector_mask` inputs contain `1` on
features and `0` elsewhere; the model's `distance` operation calculates distances
in metres. Point-source inputs contain flattened
`[xPixel, yPixel, heightMetres, ...]` triples, with pixel coordinates measured from
the northwest corner (x increases east, y increases south). Use `NaN` for missing
raster cells. If the model declares an explicit grid, its dimensions and cell size
must match `grid`.

```js
// modelJson has already been loaded or produced with JSON.stringify(model).
// This example assumes your model declares an input named "habitat".
const grid = { nrows: 100, ncols: 200, cellsize: 5 };
const inputs = { habitat: new Float64Array(grid.nrows * grid.ncols).fill(1) };
const result = await runResistancePipelineBrowserAsync(modelJson, inputs, grid);
// result.output is the final Float32Array.
// Intermediate arrays are grouped by the model's step name, then output name:
// result.layers[stepName][outputName]
```

Each named step under `nodes` in your model appears under the same name in
`result.layers`. Most steps produce one array called `data`; for example, a step
named `habitat_resistance` produces `result.layers.habitat_resistance.data`.
Steps with multiple outputs use their declared output names instead.

- [Model JSON specification](../docs/pipeline-json-spec.md)
- [Example model: horseshoe bat](../example/horseshoe-bat-model.json)
- [JavaScript function signatures and result types](index.d.ts)

## Bundler requirements

Vite, or another bundler with support for native ES modules, module Web Workers, and `.wasm?url` asset imports, is required.

## Initialization and worker calls

Wasm is initialised automatically by async rasterisation, resistance, and current solver wrappers using Web Workers.
For direct Wasm calls for fine-grained functionality (e.g. raster resizing), `await load()` is required.

## API reference

- [JavaScript declarations](index.d.ts)
- [Rust/Wasm entry points](../src/lib.rs)
