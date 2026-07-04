import { useState, useCallback, useRef } from 'react';
import { solve_connectivity } from '../lib/wasm';
import { parseAsc } from '../lib/parseAsc';
import { PRESETS } from '../lib/presets';
import MapView from '../components/MapView';
import { Spinner } from '../components/Spinner';

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
  const [ptFileReader, setPtFileReader] = useState(null);
  const resRef = useRef(null);
  const ptsRef = useRef(null);
  const [resName, setResName] = useState('—');
  const [ptName, setPtName] = useState('—');

  const loadPreset = useCallback(async (p) => {
    setStatus({ text: `loading ${p.name}...`, color: '#888' });
    setSelPreset(p.id);
    setResult(null); setTimer('');
    const [rt, pt] = await Promise.all([fetch(p.res).then(r => r.text()), fetch(p.pts).then(r => r.text())]);
    const rp = parseAsc(rt), pp = parseAsc(pt);
    setResData(rp.data); setResMeta(rp.meta);
    setPtData(pp.data); setPtMeta(pp.meta);
    setResName(p.res.split('/').pop()); setPtName(p.pts.split('/').pop());
    const nc = rp.data.reduce((c, v) => v !== rp.meta.nodata && v > 0 ? c + 1 : c, 0);
    const np = pp.data.reduce((c, v) => v !== pp.meta.nodata && v > 0 ? c + 1 : c, 0);
    setStatus({ text: `loaded ${p.name} — ${nc.toLocaleString()} cells, ${np} pts — ready`, color: '#58a6ff' });
  }, []);

  const handleResFile = useCallback(async (e) => {
    const f = e.target.files[0]; if (!f) return;
    setSelPreset(null); setResName(f.name);
    const t = await f.text(); const p = parseAsc(t);
    setResData(p.data); setResMeta(p.meta);
  }, []);
  const handlePtFile = useCallback(async (e) => {
    const f = e.target.files[0]; if (!f) return;
    setPtName(f.name);
    const t = await f.text(); const p = parseAsc(t);
    setPtData(p.data); setPtMeta(p.meta);
  }, []);

  const run = useCallback(async () => {
    if (!resData || !ptData) return;
    setLoading(true); setStatus({ text: 'solving...', color: '#f90' }); setTimer('');
    await new Promise(r => requestAnimationFrame(r));
    const t0 = performance.now();
    try {
      const nd = resMeta.nodata || -9999;
      const ptI = new Int32Array(ptData.length);
      for (let i = 0; i < ptData.length; i++) { const v = ptData[i]; ptI[i] = (v === nd || isNaN(v)) ? 0 : Math.round(v); }
      const r = JSON.parse(solve_connectivity(resData, resMeta.nrows, resMeta.ncols, nd, ptI));
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
      <div className="row">
        <button className="btn run" onClick={run} disabled={!resData || !ptData || loading}>Run</button>
        <span className="timer">{timer}</span>
        <span style={{ fontSize: '.75em', color: '#555' }}>custom:</span>
        <label className="btn" htmlFor="resFile" style={loading ? { pointerEvents: 'none', opacity: 0.5 } : {}}>+ res</label>
        <input type="file" id="resFile" accept=".asc,.txt" onChange={handleResFile} disabled={loading} style={{ display: 'none' }} />
        <span className="fname">{resName}</span>
        <label className="btn" htmlFor="ptFile" style={loading ? { pointerEvents: 'none', opacity: 0.5 } : {}}>+ pts</label>
        <input type="file" id="ptFile" accept=".asc,.txt" onChange={handlePtFile} disabled={loading} style={{ display: 'none' }} />
        <span className="fname">{ptName}</span>
      </div>
      <div className="status" style={{ color: status.color, display: 'flex', alignItems: 'center', position: 'relative', zIndex: 91 }}>
        {loading && <Spinner />}{status.text}
      </div>

      <div className="layout">
        <div className="sidebar">
          {PRESETS.map(grp => (
            <div key={grp.group} className="preset-group">
              <h3>{grp.group}</h3>
              {grp.items.map(p => (
                <div key={p.id} className={'preset' + (selPreset === p.id ? ' sel' : '')}
                  onClick={() => !loading && loadPreset(p)} style={loading ? { pointerEvents: 'none', opacity: 0.5 } : {}}>{p.name}</div>
              ))}
            </div>
          ))}
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
      {loading && <div style={{ position: 'fixed', inset: 0, zIndex: 90, cursor: 'not-allowed' }} onClick={e => e.stopPropagation()} />}
    </div>
  );
}

function ResistanceTable({ result }) {
  const { point_ids: ids, resistances: mat } = result;
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
