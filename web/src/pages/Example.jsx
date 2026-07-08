import { useState, useCallback, useRef, useEffect } from 'react';
import { run_geospatial_pipeline_cached_mg_async, reset_cache_async } from '../lib/wasm';
import { parseAsc } from '../lib/parseAsc';
import { renderMap } from '../lib/render';
import MapView from '../components/MapView';
import { StatusBar } from '../components/StatusBar';
import { ComputeModal } from '../components/ComputeModal';

const GEO_BASE = '/geodata';

function LayerMaskCanvas({ data, nrows, ncols }) {
  const canvasRef = useRef(null);
  useEffect(() => {
    const c = canvasRef.current;
    if (c && data) renderMap(c, data, nrows, ncols, -1, false);
  }, [data, nrows, ncols]);
  return <canvas ref={canvasRef} className="layer-mask-canvas" />;
}

export default function Example() {
  const [resolution, setResolution] = useState('500');
  const [baseData, setBaseData] = useState(null);
  const [baseMeta, setBaseMeta] = useState(null);
  const [srcData, setSrcData] = useState(null);
  const [gndData, setGndData] = useState(null);
  const [geojsonStr, setGeojsonStr] = useState('');
  const [result, setResult] = useState(null);
  const [resScale, setResScale] = useState('log');
  const [curScale, setCurScale] = useState('log');
  const [voltScale, setVoltScale] = useState('lin');
  const [loading, setLoading] = useState(false);
  const [computing, setComputing] = useState(false);
  const [status, setStatus] = useState('Pick a resolution and load data.');
  const [timer, setTimer] = useState('');
  const [error, setError] = useState(false);

  const [terrainRes, setTerrainRes] = useState(1.0);
  const [roadRes, setRoadRes] = useState(50);
  const [roadWidth, setRoadWidth] = useState(3);
  const [riverRes, setRiverRes] = useState(0.5);
  const [riverWidth, setRiverWidth] = useState(4);
  const [buildRes, setBuildRes] = useState(500);
  const [dirichlet, setDirichlet] = useState(true);

  const loadData = useCallback(async (res) => {
    setResolution(res);
    setResult(null); setTimer(''); setLoading(true); setError(false);
    setStatus(`loading Chudleigh ${res}x${res} data...`);

    const [baseText, srcText, gndText, geoResp] = await Promise.all([
      fetch(`${GEO_BASE}/base_resistance_${res}.asc`).then(r => r.text()),
      fetch(`${GEO_BASE}/source_${res}.asc`).then(r => r.text()),
      fetch(`${GEO_BASE}/ground_${res}.asc`).then(r => r.text()),
      fetch(`${GEO_BASE}/all_features_${res}.geojson`).then(r => r.text()),
    ]);

    const bp = parseAsc(baseText);
    setBaseData(bp.data); setBaseMeta(bp.meta);
    setSrcData(parseAsc(srcText).data);
    setGndData(parseAsc(gndText).data);
    setGeojsonStr(geoResp);

    const nc = bp.data.reduce((c, v) => v !== bp.meta.nodata && v > 0 ? c + 1 : c, 0);
    setStatus(`loaded Chudleigh ${res}x${res} — ${nc.toLocaleString()} conductive cells`);
    setLoading(false);
  }, []);

  useEffect(() => { loadData('500'); }, []);

  useEffect(() => {
    reset_cache_async().catch(() => {});
  }, [resolution, terrainRes, roadRes, roadWidth, riverRes, riverWidth, buildRes]);

  const run = useCallback(async () => {
    if (!baseData || !srcData || !geojsonStr) return;
    setComputing(true); setLoading(true); setError(false);
    setStatus('rasterising features + solving (MG CG)...'); setTimer('');
    const t0 = performance.now();
    try {
      const nd = baseMeta.nodata || -9999;
      const scaledBase = baseData.map(v => v !== nd ? v * terrainRes : v);
      const params = {
        roads:    { resistance: roadRes,    width: roadWidth },
        rivers:   { resistance: riverRes,   width: riverWidth },
        buildings:{ resistance: buildRes,   width: 0 },
      };
      const ymax = baseMeta.yllcorner + baseMeta.nrows * baseMeta.cellsize;
      const gd = gndData || new Float64Array(baseMeta.nrows * baseMeta.ncols);

      const json = await run_geospatial_pipeline_cached_mg_async(
        scaledBase, baseMeta.nrows, baseMeta.ncols, nd,
        geojsonStr, JSON.stringify(params),
        baseMeta.xllcorner, ymax, baseMeta.cellsize,
        srcData, gd, 100_000, 1e-6, dirichlet,
      );
      const r = JSON.parse(json);
      setResult(r);
      const s = ((performance.now() - t0) / 1000).toFixed(2);
      setTimer(`${s}s`);
      const iters = r.total_iters || 0;
      setStatus(`${s}s · ${iters} iters`);
    } catch (err) {
      setError(true);
      setStatus(`error: ${err.message}`);
    } finally {
      setLoading(false);
      setComputing(false);
    }
  }, [baseData, srcData, gndData, geojsonStr, baseMeta, terrainRes, roadRes, roadWidth, riverRes, riverWidth, buildRes, dirichlet]);

  const hasData = !!(baseData && srcData && geojsonStr);

  const layerMasks = result ? (result.layer_masks || []) : [];

  const numInput = (label, value, setter, min = 0.01, max = 1000, step = 0.01) => (
    <div className="example-num-wrap">
      <div className="example-num-label">{label}</div>
      <input type="number" min={min} max={max} step={step} value={value}
        onChange={e => setter(+e.target.value || min)} disabled={loading}
        className="example-num-input" />
    </div>
  );

  return (
    <div>
      <ComputeModal visible={computing} />
      <div className="row">
        <button className="btn run" onClick={run} disabled={!hasData || loading}>Run</button>
        <span className="timer">{timer}</span>
      </div>
      <StatusBar status={status} loading={loading && !computing} error={error} />

      <div className="layout">
        <div className="sidebar">
          <div className="preset-group">
            <h3>Resolution</h3>
            {['500', '1000'].map(r => (
              <div key={r} className={'preset' + (resolution === r ? ' sel' : '') + (loading ? ' loading' : '')}
                onClick={() => !loading && loadData(r)}>
                Chudleigh {r}×{r}
              </div>
            ))}
          </div>

          {hasData && (
            <div className="preset-group">
              <h3>Resistance Params</h3>
              {numInput('Terrain ×', terrainRes, setTerrainRes)}
              {numInput('Road Res', roadRes, setRoadRes)}
              {numInput('River Res', riverRes, setRiverRes)}
              {numInput('Build Res', buildRes, setBuildRes)}
              <h3>Line Widths</h3>
              {numInput('Road Width', roadWidth, setRoadWidth, 0, 20, 0.5)}
              {numInput('River Width', riverWidth, setRiverWidth, 0, 20, 0.5)}
              <h3>Solver</h3>
              <div style={{display:'flex', gap:8}}>
                <div className={'preset' + (!dirichlet ? ' sel' : '') + (loading ? ' loading' : '')}
                  onClick={() => !loading && setDirichlet(false)}>
                  Neumann ground
                </div>
                <div className={'preset' + (dirichlet ? ' sel' : '') + (loading ? ' loading' : '')}
                  onClick={() => !loading && setDirichlet(true)}>
                  Dirichlet ground (V=0)
                </div>
              </div>
            </div>
          )}
        </div>

        <div className="main">
          {!hasData && (
            <div className="example-placeholder">
              Select a resolution above to load Chudleigh data.
            </div>
          )}
          {hasData && !result && (
            <div className="example-placeholder">
              Adjust parameters and press Run to compute.
            </div>
          )}
          {result && (
            <>
              <div className="maps">
                <MapView type="res" data={result.resistance_map}
                  meta={{ nrows: result.nrows, ncols: result.ncols, nodata: baseMeta.nodata }}
                  logScale={resScale === 'log'} onToggleScale={setResScale} />
                <MapView type="cur" data={result.current_map}
                  meta={{ nrows: result.nrows, ncols: result.ncols, nodata: 0 }}
                  logScale={curScale === 'log'} onToggleScale={setCurScale} />
                <MapView type="volt" data={result.voltage_map}
                  meta={{ nrows: result.nrows, ncols: result.ncols, nodata: 0 }}
                  logScale={voltScale === 'log'} onToggleScale={setVoltScale} />
              </div>
              {layerMasks.length > 0 && (
                <div className="maps maps--auto" style={{ '--grid-cols': Math.min(layerMasks.length, 3) }}>
                  {layerMasks.map(m => (
                    <div key={m.name} className="layer-mask-card">
                      <div className="layer-mask-header">
                        {m.name} mask {result.ncols}×{result.nrows}
                      </div>
                      <LayerMaskCanvas data={m.data} nrows={result.nrows} ncols={result.ncols} />
                    </div>
                  ))}
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
