import {
  initSync,
  get_memory,
  __reset,
  solve_raster_sources_mg,
  solve_raster_sources_mg_stencil,
  solve_raster_sources_mg_mixed,
  solve_raster_sources_mg_stencil_f32,
  NamedInputArrays,
  run_resistance_pipeline_browser,
  rasterize_geojson,
} from 'lmConnect/lmConnect.js';
import wasmUrl from 'lmConnect/lmConnect_bg.wasm?url';
import { parseAsc } from 'lmConnect';
import { exampleDefaults, exampleFeatureMasks, exampleModel } from '../data';
import modelTemplate from '../../chudleigh-model.json';
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
const TOL_MIXED = 1e-3;

function solveGmg(resMap, nrows, ncols, nodata, src, gnd, useDirichlet) {
  return solve_raster_sources_mg(resMap, nrows, ncols, nodata, src, gnd, MAX_ITER, TOL, useDirichlet);
}

function solveGmgStencil(resMap, nrows, ncols, nodata, src, gnd, useDirichlet) {
  return solve_raster_sources_mg_stencil(resMap, nrows, ncols, nodata, src, gnd, MAX_ITER, TOL, useDirichlet);
}

const SOLVERS = {
  gmg: solveGmg,
  'gmg-lowmem': solveGmgStencil,
  'gmg-mixedp-lowmem': (res, nr, nc, nd, src, gnd, dirichlet) =>
    solve_raster_sources_mg_mixed(res, nr, nc, nd, src, gnd, MAX_ITER, TOL_MIXED, dirichlet),
};

const benchmarkSettings = { ...exampleDefaults(modelTemplate), terrainRes: 1 };
const benchmarkModel = exampleModel(modelTemplate, benchmarkSettings);

function benchmarkResistance(base, geojson, grid) {
  const masks = exampleFeatureMasks(geojson, grid, benchmarkSettings, (features, bounds) =>
    rasterize_geojson(JSON.stringify(features), bounds.nrows, bounds.ncols,
      bounds.xmin, bounds.ymax, bounds.cellsize, bounds.buffer));
  const inputs = new NamedInputArrays();
  try {
    inputs.insert('base', base);
    for (const [name, mask] of masks) inputs.insert(name, mask);
    const result = run_resistance_pipeline_browser(
      benchmarkModel, inputs, grid.nrows, grid.ncols, grid.cellsize,
      '{"allLayers":false}', new Float64Array(), new Float64Array(),
    );
    try {
      return result.release_f64_raster(0);
    } finally {
      result.free();
    }
  } finally {
    inputs.free();
  }
}

function runBenchmark(solveFn, precision, [baseRaster, nrows, ncols, nodata, geojson, xmin, ymax, cellsize, srcData, gndData, useDirichletGround]) {
  const ArrayType = precision !== 'f64' ? Float32Array : Float64Array;
  const t0 = performance.now();
  const resistance = benchmarkResistance(baseRaster, geojson, { nrows, ncols, xmin, ymax, cellsize });
  const prepared = precision === 'f64' ? resistance : new ArrayType(resistance);
  const t1 = performance.now();
  const prepMem = getWasmAllocatedMB();
  // Keep the prepared raster in JS and reset WASM before measuring the solve.
  // Connectivity measures a fresh instance, including input transfer and output export.
  freshInstance();
  const t2 = performance.now();
  const out = solveFn(prepared, nrows, ncols, nodata, new ArrayType(srcData), new ArrayType(gndData), !!useDirichletGround);
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
      if (!['f32', 'mixed', 'f64'].includes(precision)) throw new Error('Unknown precision');
      const scenario = SCENARIOS.find(s => s.id === scenarioId);
      if (!scenario) throw new Error('Unknown scenario');
      await getCompiledModule();
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
      freshInstance();
      const start = performance.now();
      const ArrayType = precision !== 'f64' ? Float32Array : Float64Array;
      const solve = precision === 'mixed' ? SOLVERS['gmg-mixedp-lowmem'] : precision === 'f32'
        ? (res, nr, nc, nd, src, gnd, dirichlet) => solve_raster_sources_mg_stencil_f32(res, nr, nc, nd, src, gnd, MAX_ITER, 1e-3, dirichlet)
        : solveGmgStencil;
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
      const result = runBenchmark(solver, fn.endsWith('mixedp-lowmem') ? 'mixed' : 'f64', args);
      self.postMessage({ id, result });
      return;
    }

    self.postMessage({ id, error: `unknown fn: ${fn}` });
  } catch (err) {
    console.error('benchmark worker error:', fn, err);
    self.postMessage({ id, error: err.message || String(err) });
  }
};
