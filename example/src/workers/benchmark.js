import {
  initSync,
  get_memory,
  __reset,
  solve_raster_sources_jacobi_cached,
  solve_raster_sources_mg,
  rasterize_geojson,
  reset_cache,
} from '@wasm-connect/lib/wasm_connect.js';
import wasmUrl from '@wasm-connect/lib/wasm_connect_bg.wasm?url';

let compiledModule = null;

function getWasmAllocatedMB() { return get_memory().buffer.byteLength / (1024 * 1024); }

async function getCompiledModule() {
  if (!compiledModule) {
    const resp = await fetch(wasmUrl);
    compiledModule = await WebAssembly.compile(await resp.arrayBuffer());
  }
  return compiledModule;
}

function freshInstance() {
  const m = compiledModule;
  if (!m) throw new Error('no compiled wasm module');
  __reset();
  initSync(m);
}

const MAX_ITER = 100_000;
const TOL = 1e-6;

function solveJacobi(resMap, nrows, ncols, nodata, src, gnd, useDirichlet) {
  return solve_raster_sources_jacobi_cached(resMap, nrows, ncols, nodata, src, gnd, MAX_ITER, TOL, false, useDirichlet);
}

function solveGmg(resMap, nrows, ncols, nodata, src, gnd, useDirichlet) {
  return solve_raster_sources_mg(resMap, nrows, ncols, nodata, src, gnd, MAX_ITER, TOL, useDirichlet);
}

const SOLVERS = {
  jacobi: solveJacobi,
  gmg: solveGmg,
};

function runBenchmark(solveFn, [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, useDirichletGround]) {
  const t0 = performance.now();
  const resJson = rasterize_geojson(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize);
  const t1 = performance.now();
  const prepMem = getWasmAllocatedMB();
  const parsed = JSON.parse(resJson);
  const t2 = performance.now();
  const out = solveFn(new Float64Array(parsed.resistance_map), parsed.nrows, parsed.ncols, nodata, srcData, gndData, !!useDirichletGround);
  const t3 = performance.now();
  const connMem = getWasmAllocatedMB();
  const parsed_out = JSON.parse(out);
  return {
    prepTimeMs: t1 - t0,
    prepMemMb: prepMem,
    connTimeMs: t3 - t2,
    connMemMb: connMem,
    totalIters: parsed_out.total_iters || 0,
  };
}

self.onmessage = async (e) => {
  const { id, fn, args } = e.data;

  try {
    if (fn === 'reset_cache') {
      await getCompiledModule();
      freshInstance();
      reset_cache();
      self.postMessage({ id, result: { ok: true } });
      return;
    }

    const solver = fn.startsWith('benchmark_') ? SOLVERS[fn.slice('benchmark_'.length)] : null;
    if (solver) {
      await getCompiledModule();
      freshInstance();
      const result = runBenchmark(solver, args);
      self.postMessage({ id, result });
      return;
    }

    self.postMessage({ id, error: `unknown fn: ${fn}` });
  } catch (err) {
    console.error('benchmark worker error:', fn, err);
    self.postMessage({ id, error: err.message || String(err) });
  }
};
