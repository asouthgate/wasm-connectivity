import initModule, { solve_point_sources as _solvePts, solve_raster_sources as _solveRaster, downsample_raster as _downsample, get_memory, __reset } from './wasm_connect.js';
import wasmUrl from './wasm_connect_bg.wasm?url';

let ready = false;
let wasmMemory = null;

export async function load() {
  if (!ready) {
    await initModule(wasmUrl);
    wasmMemory = get_memory();
    ready = true;
  }
}

export async function reset() {
  __reset();
  await initModule(wasmUrl);
  wasmMemory = get_memory();
}

export function solve_point_sources(resData, nrows, ncols, nodata, ptData, maxIter = 100_000, tol = 1e-6) {
  return _solvePts(resData, nrows, ncols, nodata, ptData, maxIter, tol);
}

export function solve_raster_sources(resData, nrows, ncols, nodata, srcData, gndData, maxIter = 100_000, tol = 1e-6) {
  return _solveRaster(resData, nrows, ncols, nodata, srcData, gndData, maxIter, tol);
}

export function downsample_raster(data, nrows, ncols, nodata, targetRows, targetCols) {
  return _downsample(data, nrows, ncols, nodata, targetRows, targetCols);
}

export function getWasmMemoryMB() {
  return wasmMemory ? wasmMemory.buffer.byteLength / (1024 * 1024) : null;
}

const _worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
let _reqId = 0;
const _pending = new Map();

_worker.onmessage = (e) => {
  const { id, result, error } = e.data;
  const cb = _pending.get(id);
  if (cb) { _pending.delete(id); if (error) cb.reject(error); else cb.resolve(result); }
};

function _callWorker(fn, args) {
  return new Promise((resolve, reject) => {
    const id = ++_reqId;
    _pending.set(id, { resolve, reject });
    _worker.postMessage({ id, fn, args });
  });
}

export async function solve_point_sources_async(resData, nrows, ncols, nodata, ptData, maxIter = 100_000, tol = 1e-6) {
  return _callWorker('solve_point_sources', [resData, nrows, ncols, nodata, ptData, maxIter, tol]);
}

export async function solve_raster_sources_async(resData, nrows, ncols, nodata, srcData, gndData, maxIter = 100_000, tol = 1e-6) {
  return _callWorker('solve_raster_sources', [resData, nrows, ncols, nodata, srcData, gndData, maxIter, tol]);
}

export async function run_geospatial_pipeline_async(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter = 100_000, tol = 1e-6) {
  return _callWorker('run_geospatial_pipeline', [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol]);
}

export function runBenchmark(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData) {
  return _callWorker('benchmark', [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData]);
}
