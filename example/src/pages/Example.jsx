import { useState, useRef, useEffect } from 'react';
import { runGeospatialPipelineMgAsync, runGeospatialPipelineMgStencilAsync } from '@wasm-connect/lib';
import { renderMap } from '../render';
import MapView from '../components/MapView';
import { StatusBar } from '../components/StatusBar';
import { loadExampleData } from '../data';

function LayerMaskCanvas({ data, nrows, ncols }) {
  const canvasRef = useRef(null);
  useEffect(() => {
    if (canvasRef.current && data) renderMap(canvasRef.current, data, nrows, ncols, -1, false);
  }, [data, nrows, ncols]);
  return <canvas ref={canvasRef} className="map-canvas" />;
}

export default function Example() {
  const [resolution, setResolution] = useState('500');
  const [data, setData] = useState(null);
  const [result, setResult] = useState(null);
  const [scales, setScales] = useState({ res: 'quantile', cur: 'quantile', volt: 'lin' });
  const [loading, setLoading] = useState(false);
  const [status, setStatus] = useState('Loading Chudleigh data...');
  const [error, setError] = useState(false);
  const [params, setParams] = useState({
    terrainRes: 100, roadRes: 5, roadWidth: 3,
    riverRes: 0.5, riverWidth: 4, buildRes: 500,
  });
  const [dirichlet, setDirichlet] = useState(true);
  const [precision, setPrecision] = useState('f64');
  const [lowMem, setLowMem] = useState(false);

  async function loadData(res) {
    setResolution(res); setData(null); setResult(null); setLoading(true); setError(false);
    setStatus(`loading Chudleigh ${res}x${res} data...`);
    try {
      const loaded = await loadExampleData(res);
      setData(loaded);
      const nc = loaded.baseData.reduce((c, v) => v !== loaded.baseMeta.nodata && v > 0 ? c + 1 : c, 0);
      setStatus(`loaded Chudleigh ${res}x${res}`);
    } catch (err) {
      setError(true); setStatus(`error: ${err.message}`);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => { loadData('500'); }, []);

  async function run() {
    if (!data || loading) return;
    setLoading(true); setError(false);
    setStatus('rasterising features + solving (MG CG)...');
    const t0 = performance.now();
    try {
      const { baseData, baseMeta, srcData, gndData, geojsonStr } = data;
      const { terrainRes, roadRes, roadWidth, riverRes, riverWidth, buildRes } = params;
      const nd = baseMeta.nodata ?? -9999;
      const scaledBase = baseData.map(v => v !== nd ? v * terrainRes : v);
      const layers = {
        roads: { resistance: roadRes, width: roadWidth },
        rivers: { resistance: riverRes, width: riverWidth },
        buildings: { resistance: buildRes, width: 0 },
      };
      const ymax = baseMeta.yllcorner + baseMeta.nrows * baseMeta.cellsize;
      const solve = (lowMem || precision === 'mixed') ? runGeospatialPipelineMgStencilAsync : runGeospatialPipelineMgAsync;
      const r = JSON.parse(await solve(
        scaledBase, baseMeta.nrows, baseMeta.ncols, nd,
        geojsonStr, JSON.stringify(layers), baseMeta.xllcorner, ymax, baseMeta.cellsize,
        srcData, gndData || new Float64Array(baseMeta.nrows * baseMeta.ncols),
        100_000, undefined, dirichlet, precision,
      ));
      if (r.error) throw new Error(r.error);
      setResult(r);
      setStatus(`${((performance.now() - t0) / 1000).toFixed(2)}s · ${r.total_iters || 0} iters`);
    } catch (err) {
      setError(true); setStatus(`error: ${err.message}`);
    } finally {
      setLoading(false);
    }
  }

  function numInput(label, key, min = 0.01, max = 1000, step = 0.01) {
    return <label>{label}
      <input type="number" min={min} max={max} step={step} value={params[key]}
        onChange={e => setParams({ ...params, [key]: +e.target.value || min })} />
    </label>;
  }

  return (
    <div>
      <button onClick={run} disabled={!data || loading}>Run</button>
      <StatusBar status={status} loading={loading} error={error} />
      <div className="layout">
        <fieldset className="sidebar" disabled={loading}>
          <label>Resolution
            <select value={resolution} onChange={e => loadData(e.target.value)}>
              {['500', '1000'].map(r => <option key={r} value={r}>Chudleigh {r}×{r}</option>)}
            </select>
          </label>
          {data && <>
            <h3>Resistance Params</h3>
            {numInput('Terrain ×', 'terrainRes')}
            {numInput('Road Res', 'roadRes')}
            {numInput('River Res', 'riverRes')}
            {numInput('Build Res', 'buildRes')}
            <h3>Line Widths</h3>
            {numInput('Road Width', 'roadWidth', 0, 20, 0.5)}
            {numInput('River Width', 'riverWidth', 0, 20, 0.5)}
            <h3>Solver</h3>
            <label>Ground
              <select value={String(dirichlet)} onChange={e => setDirichlet(e.target.value === 'true')}>
                <option value="false">Shunt ground</option>
                <option value="true">Dirichlet ground (V=0)</option>
              </select>
            </label>
            <label>Precision
              <select value={precision} onChange={e => {
                setPrecision(e.target.value); if (e.target.value === 'mixed') setLowMem(true); setResult(null); setError(false);
                setStatus(`Ready · ${e.target.value}`);
              }}>
                <option value="f64">f64</option><option value="f32">f32</option><option value="mixed">Mixed precision</option>
              </select>
            </label>
            <label>Memory
              <select value={String(lowMem)} disabled={precision === 'mixed'} onChange={e => setLowMem(e.target.value === 'true')}>
                <option value="false">Standard (matrix)</option>
                <option value="true">Low-memory (stencil)</option>
              </select>
            </label>
          </>}
        </fieldset>
        <div className="main">
          {!result && <p>{data ? 'Adjust parameters and press Run to compute.' : 'Select a resolution to load Chudleigh data.'}</p>}
          {result && <>
            <div className="maps">
              {[
                ['res', result.resistance_map, data.baseMeta.nodata],
                ['cur', result.current_map, 0],
                ['volt', result.voltage_map, 0],
              ].map(([type, values, nodata]) => <MapView key={type} type={type} data={values}
                meta={{ nrows: result.nrows, ncols: result.ncols, nodata }}
                quantileScale={scales[type] === 'quantile'} onToggleScale={scale => setScales({ ...scales, [type]: scale })} />)}
            </div>
            <div className="maps">
              {(result.layer_masks || []).map(m => <div key={m.name} className="map-view">
                <div className="map-header">{m.name} mask {result.ncols}×{result.nrows}</div>
                <LayerMaskCanvas data={m.data} nrows={result.nrows} ncols={result.ncols} />
              </div>)}
            </div>
          </>}
        </div>
      </div>
    </div>
  );
}
