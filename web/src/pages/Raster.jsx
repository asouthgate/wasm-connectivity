import { useState, useCallback } from 'react';
import { solve_raster_sources_async, solve_raster_sources_mg_async } from '../lib/wasm';
import { parseAsc } from '../lib/parseAsc';
import { PRESETS } from '../lib/presets';
import MapView from '../components/MapView';
import { StatusBar } from '../components/StatusBar';
import { PresetList } from '../components/PresetList';
import { ComputeModal } from '../components/ComputeModal';

const RASTER_PRESETS = PRESETS.filter(g => g.mode === 'raster');

export default function Raster() {
  const [resData, setResData] = useState(null);
  const [resMeta, setResMeta] = useState(null);
  const [srcData, setSrcData] = useState(null);
  const [srcMeta, setSrcMeta] = useState(null);
  const [gndData, setGndData] = useState(null);
  const [gndMeta, setGndMeta] = useState(null);
  const [result, setResult] = useState(null);
  const [resScale, setResScale] = useState('log');
  const [curScale, setCurScale] = useState('log');
  const [voltScale, setVoltScale] = useState('lin');
  const [loading, setLoading] = useState(false);
  const [status, setStatus] = useState({ text: 'Pick a preset to begin.', color: '#888' });
  const [timer, setTimer] = useState('');
  const [selPreset, setSelPreset] = useState(null);
  const [solver, setSolver] = useState('mg');

  const loadPreset = useCallback(async (p) => {
    setStatus({ text: `loading ${p.name}...`, color: '#888' });
    setSelPreset(p.id); setResult(null); setTimer('');

    const [rt, st, gt] = await Promise.all([
      fetch(p.res).then(r => r.text()), fetch(p.src).then(r => r.text()), fetch(p.gnd).then(r => r.text())
    ]);
    const rp = parseAsc(rt), sp = parseAsc(st), gp = parseAsc(gt);
    setResData(rp.data); setResMeta(rp.meta);
    setSrcData(sp.data); setSrcMeta(sp.meta);
    setGndData(gp.data); setGndMeta(gp.meta);

    const nc = rp.data.reduce((c, v) => v !== rp.meta.nodata && v > 0 ? c + 1 : c, 0);
    setStatus({ text: `loaded ${p.name} — ${nc.toLocaleString()} cells`, color: '#58a6ff' });
  }, []);

  const run = useCallback(async () => {
    if (!resData || !srcData) return;
    setLoading(true); setStatus({ text: 'solving...', color: '#f90' }); setTimer('');
    const t0 = performance.now();
    try {
      const nd = resMeta.nodata || -9999;
      const gd = gndData || new Float64Array(resMeta.nrows * resMeta.ncols);
      const solve = solver === 'mg' ? solve_raster_sources_mg_async : solve_raster_sources_async;
      const json = await solve(resData, resMeta.nrows, resMeta.ncols, nd, srcData, gd);
      const r = JSON.parse(json);
      setResult(r);
      const s = ((performance.now() - t0) / 1000).toFixed(1);
      setTimer(`${s}s`);
      const iters = r.total_iters || '';
      setStatus({ text: `solved in ${s}s${iters ? ` · ${iters} iters` : ''}`, color: '#3fb950' });
    } catch (err) {
      setStatus({ text: `error: ${err.message}`, color: '#f44' });
    } finally {
      setLoading(false);
    }
  }, [resData, srcData, gndData, resMeta, solver]);

  const hasData = !!(resData && srcData);

  return (
    <div>
      <ComputeModal visible={loading} />
      <div className="row">
        <button className="btn run" onClick={run} disabled={!hasData || loading}>Run</button>
        <select className="btn" value={solver} onChange={e => setSolver(e.target.value)} disabled={loading}
          style={{ background: '#1a1a1a', color: '#ccc', border: '1px solid #444', padding: '2px 4px', font: '12px monospace' }}>
          <option value="mg">MG CG</option>
          <option value="jacobi">Jacobi CG</option>
        </select>
        <span className="timer">{timer}</span>
      </div>
      <StatusBar status={status} loading={loading} />

      <div className="layout">
        <div className="sidebar">
          <PresetList presets={RASTER_PRESETS} selectedId={selPreset} onSelect={loadPreset} disabled={loading} />
        </div>
        <div className="main">
          {hasData && (
            <div className="maps" style={{ gridTemplateColumns: 'repeat(3, 1fr)' }}>
              <MapView type="res" data={resData} meta={resMeta} logScale={resScale === 'log'} onToggleScale={setResScale} />
              <MapView type="src" data={srcData} meta={{ ...srcMeta || resMeta, nodata: resMeta.nodata }} logScale={false} />
              {gndData && <MapView type="gnd" data={gndData} meta={{ ...gndMeta || resMeta, nodata: resMeta.nodata }} logScale={false} />}
            </div>
          )}
          {result && (
            <div className="maps" style={{ gridTemplateColumns: 'repeat(2, 1fr)', marginTop: 8 }}>
              <MapView type="volt" data={result.voltages} meta={{ nrows: result.nrows, ncols: result.ncols, nodata: 0 }} logScale={voltScale === 'log'} onToggleScale={setVoltScale} />
              <MapView type="cur" data={result.current_map} meta={{ nrows: result.nrows, ncols: result.ncols, nodata: 0 }} logScale={curScale === 'log'} onToggleScale={setCurScale} />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
