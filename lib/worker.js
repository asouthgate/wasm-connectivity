// Separate module loaded as a worker by wasm.js. Functions cannot be passed
// through worker messages, so this file imports the generated WASM bindings and
// dispatches requests by function name, returning results with the same request ID.
// The compiled WASM module is reused, but each request gets a fresh instance so
// WASM memory from the previous computation can be released. Results are copied
// out of that instance or transferred as buffers before the next request.

import {
  initSync,
  get_memory,
  __reset as _reset,
  solve_raster_sources_mg as _solveCurrentExplicit,
  solve_raster_sources_mg_f32 as _solveCurrentExplicitF32,
  solve_raster_sources_mg_stencil as _solveCurrent,
  solve_raster_sources_mg_stencil_f32 as _solveCurrentF32,
  solve_raster_sources_mg_mixed as _solveCurrentMixed,
  rasterize_geojson as _rasterize,
  run_resistance_pipeline_browser as _resistance,
  NamedInputArrays,
  roost_finder_compute as _roostFinder,
} from './lmConnect.js';
import * as bindings from './lmConnect.js';
import wasmUrl from './lmConnect_bg.wasm?url';

let compiledModule = null;

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
  _reset();
  initSync({ module: m });
}

async function startPipelineThreads(threadCount, workers) {
  const initializePool = Reflect.get(bindings, 'initialize_pipeline_threads');
  if (typeof initializePool !== 'function') {
    throw new Error('Wasm thread support requires the threaded build (make build)');
  }
  if (!globalThis.crossOriginIsolated) {
    throw new Error('Wasm threads require cross-origin isolation');
  }

  const memory = get_memory();
  const ready = [];
  for (let index = 0; index < threadCount; index++) {
    // Start the same worker script with a task-worker message in place of
    // a model request. Every worker receives the same Wasm memory.
    const workerUrl = self.location.href;
    const worker = new Worker(workerUrl, { type: 'module' });
    workers.push(worker);
    ready.push(new Promise((resolve, reject) => {
      worker.onmessage = ({ data }) => {
        if (data.error) reject(new Error(data.error));
        else resolve();
      };
      worker.onerror = event => reject(new Error(event.message || 'could not start a Wasm thread'));
      worker.postMessage({ fn: 'run_pipeline_thread', module: compiledModule, memory });
    }));
  }
  await Promise.all(ready);
  initializePool(threadCount);
}

// wasm-bindgen exports accept Float64Array for f64 slices. A transferred
// Float32Array would be re-interpreted incorrectly, so ensure f64 here.
function asF64(v) {
  if (v instanceof Float64Array) return v;
  if (ArrayBuffer.isView(v)) return new Float64Array(v);
  if (Array.isArray(v)) return new Float64Array(v);
  return new Float64Array(0);
}

function postResistanceResult(id, output, outputPrecision) {
  const releaseRaster = outputPrecision === 'f64'
    ? index => output.release_f64_raster(index)
    : index => output.release_and_convert_f64_raster_to_f32(index);
  try {
    let nextRasterIndex = 0;
    const transfer = [];
    const result = { nrows: output.nrows, ncols: output.ncols, layers: {} };

    const resistance = releaseRaster(nextRasterIndex);
    nextRasterIndex += 1;
    transfer.push(resistance.buffer);
    if (output.samples === 1) {
      result.output = resistance;
    } else {
      const variance = releaseRaster(nextRasterIndex);
      nextRasterIndex += 1;
      transfer.push(variance.buffer);
      result.samples = output.samples;
      result.resistance = { mean: resistance, variance };
    }

    if (output.has_current) {
      const current = releaseRaster(nextRasterIndex);
      nextRasterIndex += 1;
      transfer.push(current.buffer);
      if (output.samples === 1) {
        result.current = current;
      } else {
        const variance = releaseRaster(nextRasterIndex);
        nextRasterIndex += 1;
        transfer.push(variance.buffer);
        result.current = { mean: current, variance };
      }
    }

    for (const [step, name] of JSON.parse(output.layer_names())) {
      const raster = releaseRaster(nextRasterIndex);
      nextRasterIndex += 1;
      transfer.push(raster.buffer);
      let layer = raster;
      if (output.samples > 1) {
        const variance = releaseRaster(nextRasterIndex);
        nextRasterIndex += 1;
        transfer.push(variance.buffer);
        layer = { mean: raster, variance };
      }
      if (!Object.hasOwn(result.layers, step)) {
        Object.defineProperty(result.layers, step, { value: {}, enumerable: true });
      }
      Object.defineProperty(result.layers[step], name, { value: layer, enumerable: true });
    }
    self.postMessage({ id, result }, transfer);
  } finally { output.free(); }
}

self.onmessage = async (e) => {
  const { id, fn, args } = e.data;

  try {
    if (fn === 'run_pipeline_thread') {
      initSync({ module: e.data.module, memory: e.data.memory });
      self.postMessage({ ready: true });
      Reflect.get(bindings, 'run_pipeline_thread')();
      return;
    }

    await getCompiledModule();
    freshInstance();

    if (fn === 'solve_current') {
      const [resistance, nrows, ncols, nodata, source, ground, maxIter, tol, dirichlet, precision, lowMemory] = args;
      const asScalar = precision === 'f64' ? asF64
        : value => value instanceof Float32Array ? value : new Float32Array(value);
      const solve = precision === 'mixed' ? _solveCurrentMixed
        : precision === 'f32'
          ? (lowMemory ? _solveCurrentF32 : _solveCurrentExplicitF32)
          : (lowMemory ? _solveCurrent : _solveCurrentExplicit);
      const json = solve(asScalar(resistance), nrows, ncols, nodata,
        asScalar(source), asScalar(ground), maxIter, tol, dirichlet);
      self.postMessage({ id, result: JSON.parse(json) });
      return;
    }

    if (fn === 'roost_finder_compute') {
      const [detectorsCsv, masterCsv, sunsetCsv, gridSize, diffusivity, t0, t1] = args;
      const r = _roostFinder(
        detectorsCsv, masterCsv, sunsetCsv, gridSize, diffusivity, t0, t1,
      );
      self.postMessage({ id, result: r });
      return;
    }

    if (fn === 'rasterize_geojson') {
      const raster = _rasterize(...args);
      self.postMessage({ id, result: raster }, [raster.buffer]);
      return;
    }

    if (fn === 'run_resistance_pipeline_browser') {
      const [modelJson, namedInputs, grid, optionsJson, source, ground, outputPrecision] = args;
      const { threads = 1 } = JSON.parse(optionsJson);
      const pipelineWorkers = [];
      let result;
      try {
        if (threads > 1) await startPipelineThreads(threads, pipelineWorkers);
        const inputs = new NamedInputArrays();
        try {
          for (const [name, values] of Object.entries(namedInputs)) {
            inputs.insert(name, asF64(values));
          }
          result = _resistance(modelJson, inputs, grid.nrows, grid.ncols, grid.cellsize,
            optionsJson, asF64(source), asF64(ground));
        } finally {
          inputs.free();
        }
      } finally {
        for (const worker of pipelineWorkers) worker.terminate();
      }
      postResistanceResult(id, result, outputPrecision);
      return;
    }

    self.postMessage({ id, error: `unknown fn: ${fn}` });
  } catch (err) {
    console.error('worker error:', fn, err);
    self.postMessage({ id, error: err.message || String(err) });
  }
};
