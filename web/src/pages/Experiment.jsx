import { useState, useCallback, useRef } from 'react';
import { solve_connectivity, getWasmMemoryMB, reset } from '../lib/wasm';
import { parseAsc } from '../lib/parseAsc';
import { PRESETS } from '../lib/presets';
import { Spinner } from '../components/Spinner';

function getHeapMB() {
  if (performance.memory?.usedJSHeapSize) {
    return performance.memory.usedJSHeapSize / (1024 * 1024);
  }
  return null;
}

function stat(arr) {
  if (arr.length === 0) return {};
  const sorted = [...arr].sort((a, b) => a - b);
  const sum = sorted.reduce((a, b) => a + b, 0);
  const mean = sum / sorted.length;
  const median = sorted.length % 2 === 0
    ? (sorted[sorted.length / 2 - 1] + sorted[sorted.length / 2]) / 2
    : sorted[Math.floor(sorted.length / 2)];
  const min = sorted[0];
  const max = sorted[sorted.length - 1];
  const variance = sorted.reduce((a, b) => a + (b - mean) ** 2, 0) / sorted.length;
  const stddev = Math.sqrt(variance);
  return { mean, median, min, max, stddev };
}

function downloadCSV(runs) {
  const header = 'run,time_s,heap_before_mb,heap_after_mb,wasm_before_mb,wasm_after_mb\n';
  const rows = runs.map(r =>
    `${r.run},${r.time.toFixed(3)},${r.heapBefore ?? ''},${r.heapAfter ?? ''},${r.wasmBefore ?? ''},${r.wasmAfter ?? ''}`
  ).join('\n');
  const blob = new Blob([header + rows], { type: 'text/csv' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url; a.download = 'benchmark.csv'; a.click();
  URL.revokeObjectURL(url);
}

export default function Experiment() {
  const [resData, setResData] = useState(null);
  const [resMeta, setResMeta] = useState(null);
  const [ptData, setPtData] = useState(null);
  const [ptMeta, setPtMeta] = useState(null);
  const [selPreset, setSelPreset] = useState(null);
  const [reps, setReps] = useState(5);
  const [runs, setRuns] = useState([]);
  const [running, setRunning] = useState(false);
  const [status, setStatus] = useState({ text: 'Pick a preset and set repetitions.', color: '#888' });
  const logRef = useRef(null);

  const loadPreset = useCallback(async (p) => {
    setStatus({ text: `loading ${p.name}...`, color: '#888' });
    setSelPreset(p.id); setRuns([]);
    const [rt, pt] = await Promise.all([fetch(p.res).then(r => r.text()), fetch(p.pts).then(r => r.text())]);
    const rp = parseAsc(rt), pp = parseAsc(pt);
    setResData(rp.data); setResMeta(rp.meta);
    setPtData(pp.data); setPtMeta(pp.meta);
    const nc = rp.data.reduce((c, v) => v !== rp.meta.nodata && v > 0 ? c + 1 : c, 0);
    const np = pp.data.reduce((c, v) => v !== pp.meta.nodata && v > 0 ? c + 1 : c, 0);
    setStatus({ text: `loaded ${p.name} — ${nc.toLocaleString()} cells, ${np} pts`, color: '#58a6ff' });
  }, []);

  const start = useCallback(async () => {
    if (!resData || !ptData || running) return;
    setRunning(true); setRuns([]); setStatus({ text: 'running...', color: '#f90' });

    await new Promise(r => requestAnimationFrame(r));

    const nd = resMeta.nodata || -9999;
    const ptI = new Int32Array(ptData.length);
    for (let i = 0; i < ptData.length; i++) { const v = ptData[i]; ptI[i] = (v === nd || isNaN(v)) ? 0 : Math.round(v); }

    const results = [];
    const nrows = resMeta.nrows, ncols = resMeta.ncols;

    for (let i = 1; i <= reps; i++) {
      await reset();

      const heapBefore = getHeapMB();
      const wasmBefore = getWasmMemoryMB();
      const t0 = performance.now();
      JSON.parse(solve_connectivity(resData, nrows, ncols, nd, ptI));
      const elapsed = (performance.now() - t0) / 1000;
      const heapAfter = getHeapMB();
      const wasmAfter = getWasmMemoryMB();

      const entry = { run: i, time: elapsed, heapBefore, heapAfter, wasmBefore, wasmAfter };
      results.push(entry);
      setRuns([...results]);

      await new Promise(r => setTimeout(r, 0));
    }

    const times = results.map(r => r.time);
    const s = stat(times);
    setStatus({
      text: `done — ${reps} runs. mean: ${s.mean.toFixed(2)}s, median: ${s.median.toFixed(2)}s, min: ${s.min.toFixed(2)}s, max: ${s.max.toFixed(2)}s, std: ${s.stddev.toFixed(2)}s`,
      color: '#3fb950'
    });
    setRunning(false);
  }, [resData, ptData, resMeta, ptMeta, reps, running]);

  const hasData = !!(resData && ptData);
  const times = runs.map(r => r.time);
  const nCols = 6;

  return (
    <div>
      <h1 style={{ marginBottom: 8 }}>Experiment</h1>
      <div className="row">
        <span style={{ fontSize: '.75em', color: '#888' }}>repetitions:</span>
        <input type="number" value={reps} onChange={e => setReps(Math.max(1, Math.min(100, +e.target.value || 1)))}
          style={{ width: 50, background: '#222', border: '1px solid #444', color: '#ccc', padding: '3px 6px', font: '12px monospace' }} />
        <button className="btn run" onClick={start} disabled={!hasData || running}>Start</button>
        {runs.length > 0 && <button className="btn" onClick={() => downloadCSV(runs)} disabled={running}>Download CSV</button>}
      </div>
      <div className="status" style={{ color: status.color, display: 'flex', alignItems: 'center' }}>
        {running && <Spinner />}{status.text}
      </div>

      <div className="layout">
        <div className="sidebar">
          {PRESETS.map(grp => (
            <div key={grp.group} className="preset-group">
              <h3>{grp.group}</h3>
              {grp.items.map(p => (
                <div key={p.id} className={'preset' + (selPreset === p.id ? ' sel' : '')}
                  onClick={() => !running && loadPreset(p)}>{p.name}</div>
              ))}
            </div>
          ))}
        </div>
        <div className="main">
          {hasData && (
            <div style={{ fontSize: '.75em', color: '#888', marginBottom: 4 }}>
              {resMeta.nrows}×{resMeta.ncols}, {reps} repetition{reps !== 1 ? 's' : ''}
            </div>
          )}
          <div className="log" ref={logRef}>
            <table>
              <thead>
                <tr>
                  <th>run</th>
                  <th>time</th>
                  <th>js heap before</th>
                  <th>js heap after</th>
                  <th>wasm before</th>
                  <th>wasm after</th>
                </tr>
              </thead>
              <tbody>
                {runs.map(r => (
                  <tr key={r.run}>
                    <td>{r.run}/{reps}</td>
                    <td>{r.time.toFixed(3)}s</td>
                    <td>{r.heapBefore != null ? r.heapBefore.toFixed(1) + ' MB' : '—'}</td>
                    <td>{r.heapAfter != null ? r.heapAfter.toFixed(1) + ' MB' : '—'}</td>
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
    </div>
  );
}

function Summary({ times }) {
  const s = stat(times);
  return (
    <table>
      <tbody>
        <tr><td>mean</td><td>{s.mean.toFixed(3)}s</td></tr>
        <tr><td>median</td><td>{s.median.toFixed(3)}s</td></tr>
        <tr><td>min</td><td>{s.min.toFixed(3)}s</td></tr>
        <tr><td>max</td><td>{s.max.toFixed(3)}s</td></tr>
        <tr><td>stddev</td><td>{s.stddev.toFixed(3)}s</td></tr>
        <tr><td>n</td><td>{times.length}</td></tr>
      </tbody>
    </table>
  );
}
