import initModule, { solve_point_sources as _solvePts, solve_raster_sources as _solveRaster, get_memory, __reset } from './wasm_connect.js';
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

export function getWasmMemoryMB() {
  return wasmMemory ? wasmMemory.buffer.byteLength / (1024 * 1024) : null;
}
