# JavaScript API

This directory contains code for the JavaScript API for the
scientific `lmConnect` code.

## Build

To build the project, from the repository root directory, run:

```sh
make build
```

## Local dependency

To install this package as a local dependency, include
it in `package.json` as:

```json
{
  "dependencies": {
    "lmConnect": "file:../lib"
  }
}
```

## Minimal integration

As an example of using the bindings to run connectivity analysis,
once resistance, source, and ground f64 arrys are initialised, the
`solveCurrentAsync` function can be run as follows.
In this example, there is a single source and ground pixel, each set
to `1`.

```js
import {
  rasterizeGeojsonAsync,
  runResistancePipelineBrowserAsync,
  solveCurrentAsync,
} from 'lmConnect';

const nrows = 32;
const ncols = 32;
const size = nrows * ncols;
const noRoads = new Float32Array(size);
const noRivers = new Float32Array(size);

// One building as an example
const buildings = {
  type: 'FeatureCollection',
  features: [{
    type: 'Feature',
    properties: { value: 10 },
    geometry: {
      type: 'Polygon',
      coordinates: [[[12, 12], [20, 12], [20, 20], [12, 20], [12, 12]]],
    },
  }],
};

// Rasterise the buildings to get a buildings mask
const buildingMask = await rasterizeGeojsonAsync(buildings, {
  nrows, ncols, xmin: 0, ymax: nrows, cellsize: 1,
});

// Trivial DTM/DSM rasters
const dtm = new Float32Array(size).fill(100);
const dsm = dtm.slice();

// Define the resistance parameters used for the pipeline
const params = {
  road_buffer: 200, road_resmax: 10, road_xmax: 5,
  river_buffer: 10, river_resmax: 2000, river_xmax: 4,
  landscape_rankmax: 8, landscape_resmax: 100, landscape_xmax: 5,
  linear_buffer: 10, linear_rankmax: 4, linear_resmax: 22000, linear_xmax: 3,
  lamp_resmax: 1e8, lamp_xmax: 1, lamp_ext: 100,
  pixw: 1, nrows, ncols,
};

// Run the resistance pipeline
const resistance = await runResistancePipelineBrowserAsync(
  noRoads, noRivers, buildingMask, dtm, dsm,
  new Float32Array(size).fill(1), // Generic resistance raster.
  new Float32Array(0), // No lamp data.
  new Float32Array(size).fill(4), // Landscape conductance raster.
  JSON.stringify(params),
);

// Define a simple source and ground with one pixel each
const source = new Float64Array(size);
const ground = new Float64Array(size);
source[0] = 1;
ground[size - 1] = 1;

// Solve current and voltage from the resistance raster.
const result = await solveCurrentAsync(
  resistance.total_res, nrows, ncols, -9999,
  source, ground,
);
if (result.error) throw new Error(result.error);
console.log(result.output.current_map, result.output.voltages);
```

## Bundler requirements

Vite, or another bundler with support for native ES modules, module Web Workers, and `.wasm?url` asset imports, is required.

## Initialization and worker calls

Wasm is initialised automatically by async rasterisation, resistance, and current solver wrappers using Web Workers.
For direct Wasm calls for fine-grained functionality (e.g. raster resizing), `await load()` is required.

## API reference

- [JavaScript declarations](index.d.ts)
- [Rust/Wasm entry points](../src/lib.rs)
