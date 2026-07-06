import { useState, useCallback, useRef, useEffect } from 'react';
import { run_geospatial_pipeline_async } from '../lib/wasm';
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
  return <canvas ref={canvasRef} style={{ display: 'block', imageRendering: 'pixelated', width: '100%', height: 'auto' }} />;
}

export default function Geospatial() {
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
  const [status, setStatus] = useState({ text: 'Pick a resolution and load data.', color: '#888' });
  const [timer, setTimer] = useState('');

  const [terrainRes, setTerrainRes] = useState(1.0);
  const [roadRes, setRoadRes] = useState(50);
  const [roadWidth, setRoadWidth] = useState(3);
  const [riverRes, setRiverRes] = useState(0.5);
  const [riverWidth, setRiverWidth] = useState(4);
  const [buildRes, setBuildRes] = useState(500);

  const loadData = useCallback(async (res) => {
    setResolution(res);
    setResult(null); setTimer(''); setLoading(true);
    setStatus({ text: `loading Chudleigh ${res}x${res} data...`, color: '#888' });

    const [baseText, srcText, gndText, geoResp] = await Promise.all([
      fetch(`${GEO_BASE}/base_resistance_${res}.asc`).then(r => r.text()),
      fetch(`${GEO_BASE}/source_${res}.asc`).then(r => r.text()),
      fetch(`${GEO_BASE}/ground_${res}.asc`).then(r => r.text()),
      fetch(`${GEO_BASE}/all_features_${res}.geojson`).then(r => r.text()),
    ]);

    const bp = parseAsc(baseText);
    const sp = parseAsc(srcText);
    const gp = parseAsc(gndText);

    setBaseData(bp.data); setBaseMeta(bp.meta);
    setSrcData(sp.data);
    setGndData(gp.data);
    setGeojsonStr(geoResp);

    const nc = bp.data.reduce((c, v) => v !== bp.meta.nodata && v > 0 ? c + 1 : c, 0);
    setStatus({ text: `loaded Chudleigh ${res}x${res} — ${nc.toLocaleString()} conductive cells`, color: '#58a6ff' });
    setLoading(false);
  }, []);

  useEffect(() => { loadData('500'); }, []);

  const run = useCallback(async () => {
    if (!baseData || !srcData || !geojsonStr) return;
    setComputing(true); setLoading(true);
    setStatus({ text: 'rasterising features + solving...', color: '#f90' }); setTimer('');
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
      const json = await run_geospatial_pipeline_async(
        scaledBase, baseMeta.nrows, baseMeta.ncols, nd,
        geojsonStr, JSON.stringify(params),
        baseMeta.xllcorner, ymax, baseMeta.cellsize,
        srcData, gd,
      );
      const r = JSON.parse(json);
      setResult(r);
      const s = ((performance.now() - t0) / 1000).toFixed(1);
      setTimer(`${s}s`);
      setStatus({ text: `solved in ${s}s`, color: '#3fb950' });
    } catch (err) {
      setStatus({ text: `error: ${err.message}`, color: '#f44' });
    } finally {
      setLoading(false);
      setComputing(false);
    }
  }, [baseData, srcData, gndData, geojsonStr, baseMeta, terrainRes, roadRes, roadWidth, riverRes, riverWidth, buildRes]);

  const hasData = !!(baseData && srcData && geojsonStr);

  const layerMasks = result ? (result.layer_masks || []) : [];

  const numInput = (label, value, setter, min = 0.01, max = 1000, step = 0.01) => (
    <div style={{ marginBottom: 6 }}>
      <div style={{ fontSize: '.7em', color: '#888', marginBottom: 1 }}>{label}</div>
      <input type="number" min={min} max={max} step={step} value={value}
        onChange={e => setter(+e.target.value || min)} disabled={loading}
        style={{ width: '100%', background: '#1a1a1a', border: '1px solid #444', color: '#ccc', padding: '2px 4px', font: '12px monospace', borderRadius: 2 }} />
    </div>
  );

  return (
    <div>
      <ComputeModal visible={computing} />
      <div className="row">
        <button className="btn run" onClick={run} disabled={!hasData || loading}>Run</button>
        <span className="timer">{timer}</span>
      </div>
      <StatusBar status={status} loading={loading && !computing} />

      <div className="layout">
        <div className="sidebar">
          <div className="preset-group">
            <h3>Resolution</h3>
            {['500', '1000'].map(r => (
              <div key={r} className={'preset' + (resolution === r ? ' sel' : '')}
                onClick={() => !loading && loadData(r)}
                style={loading ? { pointerEvents: 'none', opacity: 0.5 } : {}}>
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
              <h3 style={{ marginTop: 8 }}>Line Widths</h3>
              {numInput('Road Width', roadWidth, setRoadWidth, 0, 20, 0.5)}
              {numInput('River Width', riverWidth, setRiverWidth, 0, 20, 0.5)}
            </div>
          )}
        </div>

        <div className="main">
          {!hasData && (
            <div style={{ color: '#555', padding: 40, textAlign: 'center', fontSize: '.85em' }}>
              Select a resolution above to load Chudleigh data.
            </div>
          )}
          {hasData && !result && (
            <div style={{ color: '#555', padding: 40, textAlign: 'center', fontSize: '.85em' }}>
              Adjust parameters and press <span style={{ color: '#3fb950' }}>Run</span> to compute.
            </div>
          )}
          {result && (
            <>
              <div className="maps" style={{ gridTemplateColumns: 'repeat(3, 1fr)' }}>
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
                <div className="maps" style={{ gridTemplateColumns: `repeat(${Math.min(layerMasks.length, 3)}, 1fr)`, marginTop: 8 }}>
                  {layerMasks.map(m => (
                    <div key={m.name} style={{ border: '1px solid #333' }}>
                      <div style={{ fontSize: '.65em', padding: '3px 6px', background: '#1a1a1a', borderBottom: '1px solid #333', color: '#888' }}>
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
