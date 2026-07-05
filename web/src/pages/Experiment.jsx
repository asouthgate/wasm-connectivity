import { useState, useCallback, useRef } from 'react';
import { solve_point_sources, solve_raster_sources, getWasmMemoryMB, reset } from '../lib/wasm';
import { parseAsc } from '../lib/parseAsc';
import { PRESETS } from '../lib/presets';
import { Spinner } from '../components/Spinner';

function stat(arr) {
  if (arr.length === 0) return {};
  const s = [...arr].sort((a, b) => a - b);
  const sum = s.reduce((a, b) => a + b, 0);
  const mean = sum / s.length;
  const med = s.length % 2 === 0 ? (s[s.length / 2 - 1] + s[s.length / 2]) / 2 : s[Math.floor(s.length / 2)];
  const v = s.reduce((a, b) => a + (b - mean) ** 2, 0) / s.length;
  return { mean, median: med, min: s[0], max: s[s.length - 1], stddev: Math.sqrt(v) };
}

function downloadCSV(runs) {
  const h = 'run,time_s,wasm_start_mb,wasm_end_mb,mode\n';
  const rows = runs.map(r => `${r.run},${r.time.toFixed(3)},${r.wasmBefore??''},${r.wasmAfter??''},${r.mode}`).join('\n');
  const b = new Blob([h + rows], { type: 'text/csv' });
  const u = URL.createObjectURL(b);
  const a = document.createElement('a'); a.href = u; a.download = 'benchmark.csv'; a.click();
  URL.revokeObjectURL(u);
}

export default function Experiment() {
  const [resData, setResData] = useState(null);
  const [resMeta, setResMeta] = useState(null);
  const [ptData, setPtData] = useState(null);
  const [ptMeta, setPtMeta] = useState(null);
  const [srcData, setSrcData] = useState(null);
  const [gndData, setGndData] = useState(null);
  const [selPreset, setSelPreset] = useState(null);
  const [curMode, setCurMode] = useState(null);
  const [reps, setReps] = useState(5);
  const [runs, setRuns] = useState([]);
  const [running, setRunning] = useState(false);
  const [status, setStatus] = useState({ text: 'Pick a preset and set repetitions.', color: '#888' });

  const loadPreset = useCallback(async (p, mode) => {
    setStatus({ text: `loading ${p.name}...`, color: '#888' });
    setSelPreset(p.id); setRuns([]); setCurMode(mode);
    setPtData(null); setSrcData(null); setGndData(null);

    const [rt] = await Promise.all([fetch(p.res).then(r => r.text())]);
    const rp = parseAsc(rt);
    setResData(rp.data); setResMeta(rp.meta);

    if (mode === 'pairwise') {
      const pt = await fetch(p.pts).then(r => r.text());
      const pp = parseAsc(pt);
      setPtData(pp.data); setPtMeta(pp.meta);
      const np = pp.data.reduce((c, v) => v !== pp.meta.nodata && v > 0 ? c + 1 : c, 0);
      const nc = rp.data.reduce((c, v) => v !== rp.meta.nodata && v > 0 ? c + 1 : c, 0);
      setStatus({ text: `loaded ${p.name} — ${nc.toLocaleString()} cells, ${np} pts`, color: '#58a6ff' });
    } else {
      const [st, gt] = await Promise.all([fetch(p.src).then(r => r.text()), fetch(p.gnd).then(r => r.text())]);
      setSrcData(parseAsc(st).data); setGndData(parseAsc(gt).data);
      const nc = rp.data.reduce((c, v) => v !== rp.meta.nodata && v > 0 ? c + 1 : c, 0);
      setStatus({ text: `loaded ${p.name} — ${nc.toLocaleString()} cells`, color: '#58a6ff' });
    }
  }, []);

  const start = useCallback(async () => {
    if (!resData || running) return;
    if (curMode === 'pairwise' && !ptData) return;
    if (curMode === 'raster' && (!srcData || !gndData)) return;

    setRunning(true); setRuns([]); setStatus({ text: 'running...', color: '#f90' });
    await new Promise(r => requestAnimationFrame(r));

    const nd = resMeta.nodata || -9999;
    const nr = resMeta.nrows, nc = resMeta.ncols;
    const results = [];

    for (let i = 1; i <= reps; i++) {
      await reset();
      const wb = getWasmMemoryMB();
      const t0 = performance.now();

      if (curMode === 'pairwise') {
        const ptI = new Int32Array(ptData.length);
        for (let j = 0; j < ptData.length; j++) { const v = ptData[j]; ptI[j] = (v === nd || isNaN(v)) ? 0 : Math.round(v); }
        JSON.parse(solve_point_sources(resData, nr, nc, nd, ptI));
      } else {
        JSON.parse(solve_raster_sources(resData, nr, nc, nd, srcData, gndData));
      }

      const t = (performance.now() - t0) / 1000;
      const wa = getWasmMemoryMB();
      results.push({ run: i, time: t, wasmBefore: wb, wasmAfter: wa, mode: curMode });
      setRuns([...results]);
      await new Promise(r => setTimeout(r, 0));
    }

    const ts = results.map(r => r.time);
    const s = stat(ts);
    setStatus({ text: `done — ${reps} runs. mean: ${s.mean.toFixed(2)}s, median: ${s.median.toFixed(2)}s, min: ${s.min.toFixed(2)}s, max: ${s.max.toFixed(2)}s, std: ${s.stddev.toFixed(2)}s`, color: '#3fb950' });
    setRunning(false);
  }, [resData, ptData, srcData, gndData, resMeta, reps, running, curMode]);

  const hasData = !!(resData && (curMode === 'pairwise' ? ptData : (srcData && gndData)));
  const times = runs.map(r => r.time);
  const nCols = 4;

  return (
    <div>
      <h1 style={{ marginBottom: 8 }}>Experiment</h1>
      <div className="row">
        <span style={{ fontSize: '.75em', color: '#888' }}>repetitions:</span>
        <input type="number" value={reps} onChange={e => setReps(Math.max(1, Math.min(100, +e.target.value || 1)))}
          disabled={running} style={{ width: 50, background: '#222', border: '1px solid #444', color: '#ccc', padding: '3px 6px', font: '12px monospace' }} />
        <button className="btn run" onClick={start} disabled={!hasData || running}>Start</button>
        {runs.length > 0 && <button className="btn" onClick={() => downloadCSV(runs)} disabled={running}>Download CSV</button>}
      </div>
      <div className="status" style={{ color: status.color, display: 'flex', alignItems: 'center', position: 'relative', zIndex: 91 }}>
        {running && <Spinner />}{status.text}
      </div>

      <div className="layout">
        <div className="sidebar">
          {PRESETS.map(grp => (
            <div key={grp.group} className="preset-group">
              <h3>{grp.group}</h3>
              {grp.items.map(p => (
                <div key={p.id} className={'preset' + (selPreset === p.id ? ' sel' : '')}
                  onClick={() => !running && loadPreset(p, grp.mode)}
                  style={running ? { pointerEvents: 'none', opacity: 0.5 } : {}}>{p.name}</div>
              ))}
            </div>
          ))}
        </div>
        <div className="main">
          {hasData && (
            <div style={{ fontSize: '.75em', color: '#888', marginBottom: 4 }}>
              {resMeta.nrows}×{resMeta.ncols}, {curMode}, {reps} repetition{reps !== 1 ? 's' : ''}
            </div>
          )}
          <div className="log">
            <table>
              <thead><tr><th>run</th><th>time</th><th>wasm (start)</th><th>wasm (end)</th></tr></thead>
              <tbody>
                {runs.map(r => (
                  <tr key={r.run}>
                    <td>{r.run}/{reps}</td><td>{r.time.toFixed(3)}s</td>
                    <td>{r.wasmBefore != null ? r.wasmBefore.toFixed(1) + ' MB' : '—'}</td>
                    <td>{r.wasmAfter != null ? r.wasmAfter.toFixed(1) + ' MB' : '—'}</td>
                  </tr>
                ))}
                {running && runs.length < reps && (
                  <tr><td colSpan={nCols} style={{ color: '#f90' }}><Spinner size={12} /> running run {runs.length + 1}/{reps}...</td></tr>
                )}
              </tbody>
            </table>
          </div>
          {times.length > 0 && (
            <div style={{ marginTop: 10, fontSize: '.8em' }}>
              <Summary times={times} />
            </div>
          )}
        </div>
      </div>
      {running && <div style={{ position: 'fixed', inset: 0, zIndex: 90, cursor: 'not-allowed' }} onClick={e => e.stopPropagation()} />}
    </div>
  );
}

function Summary({ times }) {
  const s = stat(times);
  return (
    <table>
      <tbody>
        {['mean','median','min','max','stddev','n'].map((k, i) => (
          <tr key={k}><td>{k}</td><td>{k === 'n' ? times.length : s[k].toFixed(3)}s</td></tr>
        ))}
      </tbody>
    </table>
  );
}
