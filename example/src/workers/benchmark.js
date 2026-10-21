import {
  initSync,
  get_memory,
  __reset,
  solve_raster_sources_jacobi,
  solve_raster_sources_mg,
  solve_raster_sources_mg_stencil,
  solve_raster_sources_mg_stencil_f32,
  rasterize_geojson_f32,
  rasterize_geojson,
} from '@wasm-connect/lib/wasm_connect.js';
import wasmUrl from '@wasm-connect/lib/wasm_connect_bg.wasm?url';
import { parseAsc } from '@wasm-connect/lib';
import { SCENARIOS, scenarioMasks, scenarioResistance } from '../scenarios';

let compiledModule = null;

// Linear-memory capacity after the run, not live heap usage. Values are MiB.
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
const TOL = 1e-4;
const TOL_F32 = 1e-3;

function solveJacobi(resMap, nrows, ncols, nodata, src, gnd, useDirichlet) {
  return solve_raster_sources_jacobi(resMap, nrows, ncols, nodata, src, gnd, MAX_ITER, TOL, useDirichlet);
}

function solveGmg(resMap, nrows, ncols, nodata, src, gnd, useDirichlet) {
  return solve_raster_sources_mg(resMap, nrows, ncols, nodata, src, gnd, MAX_ITER, TOL, useDirichlet);
}

function solveGmgStencil(resMap, nrows, ncols, nodata, src, gnd, useDirichlet) {
  return solve_raster_sources_mg_stencil(resMap, nrows, ncols, nodata, src, gnd, MAX_ITER, TOL, useDirichlet);
}

const SOLVERS = {
  jacobi: solveJacobi,
  gmg: solveGmg,
  'gmg-lowmem': solveGmgStencil,
  'gmg-lowmem-f32': (res, nr, nc, nd, src, gnd, dirichlet) =>
    solve_raster_sources_mg_stencil_f32(res, nr, nc, nd, src, gnd, MAX_ITER, TOL_F32, dirichlet),
};

function runBenchmark(solveFn, precision, [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, useDirichletGround]) {
  const ArrayType = precision === 'f32' ? Float32Array : Float64Array;
  const rasterize = precision === 'f32' ? rasterize_geojson_f32 : rasterize_geojson;
  const t0 = performance.now();
  const resJson = rasterize(new ArrayType(baseRaster), nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize);
  const t1 = performance.now();
  const prepMem = getWasmAllocatedMB();
  const parsed = JSON.parse(resJson);
  const t2 = performance.now();
  const out = solveFn(new ArrayType(parsed.resistance_map), parsed.nrows, parsed.ncols, nodata, new ArrayType(srcData), new ArrayType(gndData), !!useDirichletGround);
  const t3 = performance.now();
  const connMem = getWasmAllocatedMB();
  const parsed_out = JSON.parse(out);
  if (parsed_out.error) throw new Error(parsed_out.error);
  return {
    prepTimeMs: t1 - t0,
    prepMemMb: prepMem,
    connTimeMs: t3 - t2,
    connMemMb: connMem,
    totalIters: parsed_out.total_iters || 0,
  };
}

async function loadScenarioRaster(name) {
  const response = await fetch(`/geodata/scenarios/${name}.asc`);
  if (!response.ok) throw new Error(`Could not load ${name}: HTTP ${response.status}`);
  const raster = parseAsc(await response.text());
  if (raster.meta.nrows !== 2000 || raster.meta.ncols !== 2000 || raster.data.length !== 4_000_000) {
    throw new Error(`${name} must contain a complete 2000×2000 raster`);
  }
  return raster;
}

self.onmessage = async (e) => {
  const { id, fn, args } = e.data;

  try {
    if (fn === 'benchmark_scenario') {
      const [scenarioId, precision = 'f64'] = args;
      if (!['f32', 'f64'].includes(precision)) throw new Error('Unknown precision');
      const scenario = SCENARIOS.find(s => s.id === scenarioId);
      if (!scenario) throw new Error('Unknown scenario');
      await getCompiledModule();
      freshInstance();
      const { data, meta } = await loadScenarioRaster(scenarioId);
      const inputs = {};
      for (const name of scenario.inputs || []) {
        const raster = await loadScenarioRaster(name);
        for (const key of ['xllcorner', 'yllcorner', 'cellsize']) {
          if (raster.meta[key] !== meta[key]) throw new Error(`${name} does not align with ${scenarioId}`);
        }
        inputs[name] = raster;
      }
      scenarioResistance(scenarioId, data, meta.nodata);
      const { source, ground } = scenarioMasks(scenarioId, data, meta.nrows, meta.ncols, meta.nodata, inputs);
      const start = performance.now();
      const ArrayType = precision === 'f32' ? Float32Array : Float64Array;
      const solve = precision === 'f32' ? SOLVERS['gmg-lowmem-f32'] : solveGmgStencil;
      const out = solve(new ArrayType(data), meta.nrows, meta.ncols, meta.nodata, new ArrayType(source), new ArrayType(ground), false);
      const connTimeMs = performance.now() - start;
      const connMemMb = getWasmAllocatedMB();
      const parsed = JSON.parse(out);
      if (parsed.error) throw new Error(parsed.error);
      for (let i = 0; i < data.length; i++) {
        if (!Number.isFinite(data[i]) || data[i] === meta.nodata || data[i] <= 0) {
          parsed.output.current_map[i] = meta.nodata;
        }
      }
      const currentMap = {
        data: parsed.output.current_map,
        nrows: parsed.output.nrows,
        ncols: parsed.output.ncols,
        nodata: meta.nodata,
      };
      self.postMessage({ id, result: { connTimeMs, connMemMb, totalIters: parsed.total_iters || 0, currentMap } });
      return;
    }

    const solver = fn.startsWith('benchmark_') ? SOLVERS[fn.slice('benchmark_'.length)] : null;
    if (solver) {
      await getCompiledModule();
      freshInstance();
      const result = runBenchmark(solver, fn.endsWith('-f32') ? 'f32' : 'f64', args);
      self.postMessage({ id, result });
      return;
    }

    self.postMessage({ id, error: `unknown fn: ${fn}` });
  } catch (err) {
    console.error('benchmark worker error:', fn, err);
    self.postMessage({ id, error: err.message || String(err) });
  }
};
