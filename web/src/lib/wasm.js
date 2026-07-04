import initModule, { solve_connectivity as _solve, solve_advanced as _solveAdv, get_memory, __reset } from './wasm_connect.js';
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

export function solve_connectivity(resData, nrows, ncols, nodata, ptData) {
  return _solve(resData, nrows, ncols, nodata, ptData);
}

export function solve_advanced(resData, nrows, ncols, nodata, srcData, gndData) {
  return _solveAdv(resData, nrows, ncols, nodata, srcData, gndData);
}

export function getWasmMemoryMB() {
  return wasmMemory ? wasmMemory.buffer.byteLength / (1024 * 1024) : null;
}
