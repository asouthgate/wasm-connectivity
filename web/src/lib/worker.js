import initModule, {
  run_geospatial_pipeline as _geo,
  run_geospatial_pipeline_cached as _geoC,
  run_geospatial_pipeline_cached_mg as _geoCmg,
  solve_point_sources as _pts,
  solve_raster_sources as _rast,
  solve_raster_sources_cached as _rastC,
  solve_raster_sources_mg as _rastMg,
  rasterize_geojson as _rasterize,
  reset_cache as _resetCache,
  get_memory,
  __reset,
} from './wasm_connect.js';
import wasmUrl from './wasm_connect_bg.wasm?url';

let ready = false;
function mb() { return get_memory().buffer.byteLength / (1024 * 1024); }
async function ensure() { if (!ready) { await initModule(wasmUrl); ready = true; } }

const FN_MAP = {
  run_geospatial_pipeline: _geo,
  solve_point_sources: _pts,
  solve_raster_sources: _rast,
  solve_raster_sources_mg: _rastMg,
};

self.onmessage = async (e) => {
  const { id, fn, args } = e.data;

  try {
    if (fn === 'reset_cache') {
      await ensure();
      _resetCache();
      self.postMessage({ id, result: { ok: true } });
      return;
    }

    if (fn === 'run_geospatial_pipeline_cached') {
      await ensure();
      const [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol, rebuildLaplacian] = args;
      const r = _geoC(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol, rebuildLaplacian);
      self.postMessage({ id, result: r });
      return;
    }

    if (fn === 'run_geospatial_pipeline_cached_mg') {
      await ensure();
      const [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol] = args;
      const r = _geoCmg(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol);
      self.postMessage({ id, result: r });
      return;
    }

    if (fn === 'benchmark_cold') {
      __reset();
      await initModule(wasmUrl);
      ready = true;

      const [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData] = args;
      const t0 = performance.now();
      const resJson = _rasterize(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize);
      const t1 = performance.now();
      const prepMem = mb();
      const parsed = JSON.parse(resJson);
      const t2 = performance.now();
      const out = _rastC(new Float64Array(parsed.resistance_map), parsed.nrows, parsed.ncols, nodata, sourceData, gndData, 100_000, 1e-6, false);
      const t3 = performance.now();
      const connMem = mb();
      const parsed_out = JSON.parse(out);
      const result = {
        prepTimeMs: t1 - t0,
        connTimeMs: t3 - t2,
        prepMemMb: prepMem,
        connMemMb: connMem,
        totalIters: parsed_out.total_iters || 0,
      };
      self.postMessage({ id, result });
      return;
    }

    if (fn === 'benchmark_warm') {
      await ensure();
      const [srcData, gndData, nrows, ncols, nodata] = args;
      const t2 = performance.now();
      const out = _rastC(new Float64Array(0), nrows, ncols, nodata, srcData, gndData, 100_000, 1e-6, false);
      const t3 = performance.now();
      const connMem = mb();
      const parsed = JSON.parse(out);
      const result = {
        prepTimeMs: 0,
        connTimeMs: t3 - t2,
        prepMemMb: 0,
        connMemMb: connMem,
        totalIters: parsed.total_iters || 0,
      };
      self.postMessage({ id, result });
      return;
    }

    if (fn === 'benchmark_hot') {
      await ensure();
      const [baseRaster, nrows, ncols, nodata, hotGeojson, layerParamsStr, xmin, ymax, cellsize, srcData, gndData] = args;
      const t0 = performance.now();
      const resJson = _rasterize(baseRaster, nrows, ncols, nodata, hotGeojson, layerParamsStr, xmin, ymax, cellsize);
      const t1 = performance.now();
      const prepMem = mb();
      const parsed = JSON.parse(resJson);
      const t2 = performance.now();
      const out = _rastC(new Float64Array(parsed.resistance_map), parsed.nrows, parsed.ncols, nodata, srcData, gndData, 100_000, 1e-6, true);
      const t3 = performance.now();
      const connMem = mb();
      const parsed_out = JSON.parse(out);
      const result = {
        prepTimeMs: t1 - t0,
        connTimeMs: t3 - t2,
        prepMemMb: prepMem,
        connMemMb: connMem,
        totalIters: parsed_out.total_iters || 0,
      };
      self.postMessage({ id, result });
      return;
    }

    await ensure();
    if (FN_MAP[fn]) {
      const r = FN_MAP[fn](...args);
      self.postMessage({ id, result: r });
      return;
    }
    self.postMessage({ id, error: `unknown fn: ${fn}` });
  } catch (err) {
    self.postMessage({ id, error: err.message || String(err) });
  }
};