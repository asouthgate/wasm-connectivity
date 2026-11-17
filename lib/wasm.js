// Public JavaScript interface. Compute calls run in a worker so they do not block
// the browser UI. new Worker(new URL('./worker.js', import.meta.url), ...) loads
// a separate module with its own execution context, so calls cross that boundary
// as messages containing a function name, arguments, and request ID. worker.js
// dispatches each call and returns its result or error to the matching promise.
// Direct helpers such as raster resizing use this module's own WASM instance.

import initModule from './lmConnect.js';
import wasmUrl from './lmConnect_bg.wasm?url';

let ready = false;

export async function load() {
  if (!ready) {
    await initModule(wasmUrl);
    ready = true;
  }
}

export { downsample_raster as downsampleRaster } from './lmConnect.js';
export { upsample_raster as upsampleRaster } from './lmConnect.js';

// Lazily construct the module worker on first use. Deferring construction
// keeps importing this module free of browser-only side effects (e.g. in
// Node/jsdom test environments) until a compute call is actually made.
let _worker = null;
let _reqId = 0;
const _pending = new Map();

function _getWorker() {
  if (!_worker) {
    _worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
    _worker.onmessage = (e) => {
      const { id, result, error } = e.data;
      const cb = _pending.get(id);
      if (cb) { _pending.delete(id); if (error) cb.reject(error); else cb.resolve(result); }
    };
    _worker.onerror = (e) => {
      const err = e.message || String(e.error || 'worker error');
      for (const [, cb] of _pending) cb.reject(new Error(err));
      _pending.clear();
    };
  }
  return _worker;
}

// transfer lists buffers sent to the worker without copying. Their ownership moves
// to the worker, making those buffers unusable here; omit it to copy the data.
function _callWorker(fn, args, transfer = []) {
  return new Promise((resolve, reject) => {
    const id = ++_reqId;
    _pending.set(id, { resolve, reject });
    try {
      _getWorker().postMessage({ id, fn, args }, transfer);
    } catch (error) {
      _pending.delete(id);
      reject(error);
    }
  });
}

export async function solveCurrentAsync(resistance, nrows, ncols, nodata, source, ground, {
  maxIter = 50_000, tol, useDirichletGround = false, precision = 'f64',
} = {}) {
  if (!['f32', 'f64', 'mixed'].includes(precision)) throw new Error('precision must be f32, f64 or mixed');
  if (tol === undefined) tol = precision === 'f64' ? 1e-4 : 1e-3;
  return _callWorker('solve_current', [resistance, nrows, ncols, nodata,
    source, ground, maxIter, tol, useDirichletGround, precision]);
}

function geospatialOptions({
  geojson = { type: 'FeatureCollection', features: [] },
  layerParams = {}, maxIter = 50_000, tol,
  useDirichletGround = false, precision = 'f64',
} = {}) {
  if (!['f32', 'f64', 'mixed'].includes(precision)) throw new Error('precision must be f32, f64 or mixed');
  if (tol === undefined) tol = precision === 'f64' ? 1e-4 : 1e-3;
  return [JSON.stringify(geojson), JSON.stringify(layerParams), maxIter, tol,
    useDirichletGround, precision];
}

export async function runGeospatialPipelineMgAsync(baseRaster, nrows, ncols, nodata, xmin, ymax, cellsize, srcData, gndData, options) {
  const [geojsonStr, layerParamsStr, maxIter, tol, useDirichletGround, precision] = geospatialOptions(options);
  const args = [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol, useDirichletGround];
  return _callWorker(precision === 'mixed' ? 'run_geospatial_pipeline_mg_mixed' : precision === 'f32' ? 'run_geospatial_pipeline_mg_f32' : 'run_geospatial_pipeline_mg', args);
}

export async function runGeospatialPipelineMgStencilAsync(baseRaster, nrows, ncols, nodata, xmin, ymax, cellsize, srcData, gndData, options) {
  const [geojsonStr, layerParamsStr, maxIter, tol, useDirichletGround, precision] = geospatialOptions(options);
  const args = [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol, useDirichletGround];
  return _callWorker(precision === 'mixed' ? 'run_geospatial_pipeline_mg_mixed' : precision === 'f32' ? 'run_geospatial_pipeline_mg_stencil_f32' : 'run_geospatial_pipeline_mg_stencil', args);
}

export async function rasterizeGeojsonAsync(features, { nrows, ncols, xmin, ymax, cellsize, buffer = 0 }) {
  if (!Number.isInteger(nrows) || nrows <= 0 || !Number.isInteger(ncols) || ncols <= 0) {
    throw new Error('nrows and ncols must be positive integers');
  }
  if (![xmin, ymax, cellsize, buffer].every(Number.isFinite) || cellsize <= 0 || buffer < 0) {
    throw new Error('grid coordinates must be finite, cellsize positive, and buffer nonnegative');
  }
  return _callWorker('rasterize_geojson', [JSON.stringify(features), nrows, ncols, xmin, ymax, cellsize, buffer]);
}

// Copy inputs before transfer to avoid detaching caller-owned buffers.
export async function runResistancePipelineBrowserAsync(modelJson, inputs, grid, options = {}) {
  const samples = options.samples ?? 1;
  if (!Number.isInteger(samples) || samples < 1) {
    throw new Error('samples must be a positive integer');
  }
  if (options.seed !== undefined && (!Number.isSafeInteger(options.seed) || options.seed < 0)) {
    throw new Error('seed must be a nonnegative safe integer');
  }
  const copies = new Map();
  const copy = input => {
    if (!copies.has(input)) {
      copies.set(input, ArrayBuffer.isView(input) ? input.slice() : new Float64Array(input));
    }
    return copies.get(input);
  };
  const inputEntries = Object.entries(inputs);
  const copiedInputs = inputEntries.map(([name, input]) => [name, copy(input)]);
  const namedInputs = Object.fromEntries(copiedInputs);
  const { currentSolverOptions, ...pipelineExecutionOptions } = options;
  const settings = { samples, allLayers: samples === 1, ...pipelineExecutionOptions };
  let source = [], ground = [];
  if (currentSolverOptions) {
    const { source: sourceInput, ground: groundInput, ...solver } = currentSolverOptions;
    source = sourceInput;
    ground = groundInput;
    settings.currentSolverOptions = solver;
  }
  const args = [modelJson, namedInputs, grid, JSON.stringify(settings), copy(source), copy(ground)];
  return _callWorker('run_resistance_pipeline_browser', args,
    [...new Set([...copies.values()].map(input => input.buffer))]);
}

export async function roostFinderComputeAsync(detectorsCsv, masterCsv, sunsetCsv, gridSize, diffusivity, t0, t1) {
  const args = [detectorsCsv, masterCsv, sunsetCsv, gridSize, diffusivity, t0, t1];
  return _callWorker('roost_finder_compute', args);
}
