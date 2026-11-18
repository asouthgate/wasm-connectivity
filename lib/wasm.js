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

function _createWorker() {
  return new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
}

function _getWorker() {
  if (!_worker) {
    _worker = _createWorker();
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

function _callThreadedPipeline(args, transfer) {
  return new Promise((resolve, reject) => {
    // The pool is tied to this instance's memory. Terminate its coordinator
    // after the run to release both the worker pool and shared memory.
    const worker = _createWorker();
    worker.onmessage = ({ data: { result, error } }) => {
      worker.terminate();
      if (error) reject(new Error(error));
      else resolve(result);
    };
    worker.onerror = event => {
      worker.terminate();
      reject(new Error(event.message || 'threaded pipeline worker failed'));
    };
    try {
      worker.postMessage({ id: 0, fn: 'run_resistance_pipeline_browser', args }, transfer);
    } catch (error) {
      worker.terminate();
      reject(error);
    }
  });
}

export async function solveCurrentAsync(resistance, nrows, ncols, nodata, source, ground, {
  maxIter = 50_000, tol, useDirichletGround = false, precision = 'f64', lowMemory = true,
} = {}) {
  if (!['f32', 'f64', 'mixed'].includes(precision)) throw new Error('precision must be f32, f64 or mixed');
  if (tol === undefined) tol = precision === 'f64' ? 1e-4 : 1e-3;
  return _callWorker('solve_current', [resistance, nrows, ncols, nodata,
    source, ground, maxIter, tol, useDirichletGround, precision, lowMemory]);
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
  const threads = options.threads ?? 1;
  if (!Number.isSafeInteger(threads) || threads < 1 || threads > 0xffffffff) {
    throw new Error('threads must be a positive 32-bit integer');
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
  const { currentSolverOptions, outputPrecision = 'f32', ...pipelineExecutionOptions } = options;
  if (outputPrecision !== 'f32' && outputPrecision !== 'f64') {
    throw new Error('outputPrecision must be f32 or f64');
  }
  const settings = { samples, allLayers: samples === 1, ...pipelineExecutionOptions, threads };
  let source = [], ground = [];
  if (currentSolverOptions) {
    const { source: sourceInput, ground: groundInput, ...solver } = currentSolverOptions;
    source = sourceInput;
    ground = groundInput;
    settings.currentSolverOptions = solver;
  }
  const args = [modelJson, namedInputs, grid, JSON.stringify(settings), copy(source), copy(ground), outputPrecision];
  const transfer = [...new Set([...copies.values()].map(input => input.buffer))];
  return threads > 1
    ? _callThreadedPipeline(args, transfer)
    : _callWorker('run_resistance_pipeline_browser', args, transfer);
}

export async function roostFinderComputeAsync(detectorsCsv, masterCsv, sunsetCsv, gridSize, diffusivity, t0, t1) {
  const args = [detectorsCsv, masterCsv, sunsetCsv, gridSize, diffusivity, t0, t1];
  return _callWorker('roost_finder_compute', args);
}
