import {
  initSync,
  run_geospatial_pipeline_cached_mg as _geoCmg,
  solve_raster_sources_cached as _rastC,
  solve_raster_sources_mg as _rastMg,
  solve_raster_sources_mg_alcouffe as _rastMgAlcouffe,
  rasterize_geojson as _rasterize,
  reset_cache as _resetCache,
  get_memory,
  __reset,
} from './wasm_connect.js';
import wasmUrl from './wasm_connect_bg.wasm?url';

let compiledModule = null;

function mb() { return get_memory().buffer.byteLength / (1024 * 1024); }

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

self.onmessage = async (e) => {
  const { id, fn, args } = e.data;

  try {
    if (fn === 'reset_cache') {
      await getCompiledModule();
      freshInstance();
      _resetCache();
      self.postMessage({ id, result: { ok: true } });
      return;
    }

    if (fn === 'run_geospatial_pipeline_cached_mg') {
      await getCompiledModule();
      freshInstance();
      const [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol] = args;
      const r = _geoCmg(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol);
      self.postMessage({ id, result: r });
      return;
    }

    if (fn === 'benchmark_jacobi') {
      await getCompiledModule();
      freshInstance();
      const [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData] = args;
      const t0 = performance.now();
      const resJson = _rasterize(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize);
      const t1 = performance.now();
      const prepMem = mb();
      const parsed = JSON.parse(resJson);
      const t2 = performance.now();
      const out = _rastC(new Float64Array(parsed.resistance_map), parsed.nrows, parsed.ncols, nodata, srcData, gndData, 100_000, 1e-6, false);
      const t3 = performance.now();
      const connMem = mb();
      const parsed_out = JSON.parse(out);
      self.postMessage({ id, result: {
        prepTimeMs: t1 - t0,
        connTimeMs: t3 - t2,
        prepMemMb: prepMem,
        connMemMb: connMem,
        totalIters: parsed_out.total_iters || 0,
      }});
      return;
    }

    if (fn === 'benchmark_gmg') {
      await getCompiledModule();
      freshInstance();
      const [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData] = args;
      const t0 = performance.now();
      const resJson = _rasterize(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize);
      const t1 = performance.now();
      const prepMem = mb();
      const parsed = JSON.parse(resJson);
      const t2 = performance.now();
      const out = _rastMg(new Float64Array(parsed.resistance_map), parsed.nrows, parsed.ncols, nodata, srcData, gndData, 100_000, 1e-6);
      const t3 = performance.now();
      const connMem = mb();
      const parsed_out = JSON.parse(out);
      self.postMessage({ id, result: {
        prepTimeMs: t1 - t0,
        connTimeMs: t3 - t2,
        prepMemMb: prepMem,
        connMemMb: connMem,
        totalIters: parsed_out.total_iters || 0,
      }});
      return;
    }

    if (fn === 'benchmark_alcouffe') {
      await getCompiledModule();
      freshInstance();
      const [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData] = args;
      const t0 = performance.now();
      const resJson = _rasterize(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize);
      const t1 = performance.now();
      const prepMem = mb();
      const parsed = JSON.parse(resJson);
      const t2 = performance.now();
      const out = _rastMgAlcouffe(new Float64Array(parsed.resistance_map), parsed.nrows, parsed.ncols, nodata, srcData, gndData, 100_000, 1e-6);
      const t3 = performance.now();
      const connMem = mb();
      const parsed_out = JSON.parse(out);
      self.postMessage({ id, result: {
        prepTimeMs: t1 - t0,
        connTimeMs: t3 - t2,
        prepMemMb: prepMem,
        connMemMb: connMem,
        totalIters: parsed_out.total_iters || 0,
      }});
      return;
    }

    self.postMessage({ id, error: `unknown fn: ${fn}` });
  } catch (err) {
    console.error('benchmark worker error:', fn, err);
    self.postMessage({ id, error: err.message || String(err) });
  }
};
