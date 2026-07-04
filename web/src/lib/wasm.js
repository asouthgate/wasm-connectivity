import initModule, { solve_connectivity as _solve } from './wasm_connect.js';
import wasmUrl from './wasm_connect_bg.wasm?url';

let ready = false;

export async function load() {
  if (!ready) {
    await initModule(wasmUrl);
    ready = true;
  }
}

export function solve_connectivity(resData, nrows, ncols, nodata, ptData) {
  return _solve(resData, nrows, ncols, nodata, ptData);
}
