import initModule, { solve_point_sources as _solvePts, solve_raster_sources as _solveRaster, solve_geospatial as _solveGeo, downsample_raster as _downsample, get_memory, __reset } from './wasm_connect.js';
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

export function solve_geospatial(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter = 100_000, tol = 1e-6) {
  return _solveGeo(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol);
}

export function downsample_raster(data, nrows, ncols, nodata, targetRows, targetCols) {
  return _downsample(data, nrows, ncols, nodata, targetRows, targetCols);
}

export function getWasmMemoryMB() {
  return wasmMemory ? wasmMemory.buffer.byteLength / (1024 * 1024) : null;
}
