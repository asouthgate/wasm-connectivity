import { useState, useCallback } from 'react';
import { solve_point_sources_async } from '../lib/wasm';
import { parseAsc } from '../lib/parseAsc';
import { PRESETS } from '../lib/presets';
import MapView from '../components/MapView';
import { StatusBar } from '../components/StatusBar';
import { PresetList } from '../components/PresetList';
import { ComputeModal } from '../components/ComputeModal';

const PAIRWISE = PRESETS.filter(g => g.mode === 'pairwise');

export default function Solver() {
  const [resData, setResData] = useState(null);
  const [resMeta, setResMeta] = useState(null);
  const [ptData, setPtData] = useState(null);
  const [ptMeta, setPtMeta] = useState(null);
  const [result, setResult] = useState(null);
  const [resScale, setResScale] = useState('log');
  const [curScale, setCurScale] = useState('log');
  const [loading, setLoading] = useState(false);
  const [status, setStatus] = useState({ text: 'Pick a preset to begin.', color: '#888' });
  const [timer, setTimer] = useState('');
  const [selPreset, setSelPreset] = useState(null);

  const loadPreset = useCallback(async (p) => {
    setStatus({ text: `loading ${p.name}...`, color: '#888' });
    setSelPreset(p.id);
    setResult(null); setTimer('');
    const [rt, pt] = await Promise.all([fetch(p.res).then(r => r.text()), fetch(p.pts).then(r => r.text())]);
    const rp = parseAsc(rt), pp = parseAsc(pt);
    setResData(rp.data); setResMeta(rp.meta);
    setPtData(pp.data); setPtMeta(pp.meta);
    const nc = rp.data.reduce((c, v) => v !== rp.meta.nodata && v > 0 ? c + 1 : c, 0);
    const np = pp.data.reduce((c, v) => v !== pp.meta.nodata && v > 0 ? c + 1 : c, 0);
    setStatus({ text: `loaded ${p.name} — ${nc.toLocaleString()} cells, ${np} pts — ready`, color: '#58a6ff' });
  }, []);

  const run = useCallback(async () => {
    if (!resData || !ptData) return;
    setLoading(true); setStatus({ text: 'solving...', color: '#f90' }); setTimer('');
    const t0 = performance.now();
    try {
      const nd = resMeta.nodata || -9999;
      const ptI = new Int32Array(ptData.length);
      for (let i = 0; i < ptData.length; i++) { const v = ptData[i]; ptI[i] = (v === nd || isNaN(v)) ? 0 : Math.round(v); }
      const json = await solve_point_sources_async(resData, resMeta.nrows, resMeta.ncols, nd, ptI);
      const r = JSON.parse(json);
      setResult(r);
      const s = ((performance.now() - t0) / 1000).toFixed(1);
      setTimer(`${s}s`);
      setStatus({ text: `solved in ${s}s`, color: '#3fb950' });
    } catch (err) {
      setStatus({ text: `error: ${err.message}`, color: '#f44' });
    } finally {
      setLoading(false);
    }
  }, [resData, ptData, resMeta]);

  return (
    <div>
      <ComputeModal visible={loading} />
      <div className="row">
        <button className="btn run" onClick={run} disabled={!resData || !ptData || loading}>Run</button>
        <span className="timer">{timer}</span>
      </div>
      <StatusBar status={status} loading={loading} />

      <div className="layout">
        <div className="sidebar">
          <PresetList presets={PAIRWISE} selectedId={selPreset} onSelect={loadPreset} disabled={loading} />
        </div>
        <div className="main">
          {(resMeta || ptMeta) && (
            <div className="maps">
              {resMeta && <MapView type="res" data={resData} meta={resMeta} logScale={resScale === 'log'} onToggleScale={setResScale} />}
              {ptMeta && <MapView type="points" data={ptData} meta={ptMeta} />}
              {result && <MapView type="cur" data={result.current_map} meta={{ nrows: result.nrows, ncols: result.ncols, nodata: 0 }} logScale={curScale === 'log'} onToggleScale={setCurScale} />}
            </div>
          )}
          {result && <ResistanceTable result={result} />}
        </div>
      </div>
    </div>
  );
}

function ResistanceTable({ result }) {
  const { point_ids: ids, resistance_matrix: mat } = result;
  if (!ids || ids.length === 0) return <div style={{ color: '#888', fontSize: '.78em', marginTop: 10 }}>No focal points found in conductive cells.</div>;
  return (
    <div className="resistances">
      <table>
        <tbody>
          <tr><th></th>{ids.map(id => <th key={id}>pt {id}</th>)}</tr>
          {mat.map((row, i) => (
            <tr key={i}><th>pt {ids[i]}</th>{row.map((v, j) => <td key={j}>{v >= 0 ? v.toFixed(4) : '—'}</td>)}</tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
