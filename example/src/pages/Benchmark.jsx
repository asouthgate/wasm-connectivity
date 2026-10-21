import { useState, useCallback, useEffect } from 'react';
import { downsampleRaster, upsampleRaster } from '@wasm-connect/lib';
import { StatusBar } from '../components/StatusBar';
import { downloadCSV } from '../stats';
import { loadExampleData } from '../data';

const RESOLUTIONS = [2000, 1800, 1600, 1400, 1200, 1000, 800, 600, 400, 200];

const DEFAULT_PARAMS = {
  roads:    { resistance: 5,  width: 3 },
  rivers:   { resistance: 0.5, width: 4 },
  buildings:{ resistance: 500, width: 0 },
};

const RUNS = ['gmg', 'gmg-lowmem'];

const RUN_LABELS = {
  gmg: 'GMG CG',
  'gmg-lowmem': 'GMG CG (low-mem)',
};

export const BENCHMARK_HEADERS = [
  'resolution',
  'repeat',
  'run',
  'prep_time_s',
  'prep_mem_mb',
  'conn_time_s',
  'conn_mem_mb',
  'total_iters'  
];

function buildBenchmarkCSV(runs) {
  const h = BENCHMARK_HEADERS.join(',') + '\n';
  const rows = runs.map(r =>
    `${r.resolution},${r.repeat},${r.run},${(r.prepTimeMs/1000).toFixed(4)},${r.prepMemMb.toFixed(2)},${(r.connTimeMs/1000).toFixed(4)},${r.connMemMb.toFixed(2)},${r.totalIters||0}`
  ).join('\n');
  downloadCSV('benchmark.csv', h, rows);
}

const _benchWorker = new Worker(new URL('../workers/benchmark.js', import.meta.url), { type: 'module' });
let _reqId = 0;
const _pending = new Map();

_benchWorker.onmessage = (e) => {
  const { id, result, error } = e.data;
  const cb = _pending.get(id);
  if (cb) { _pending.delete(id); if (error) cb.reject(error); else cb.resolve(result); }
};

function callBenchWorker(fn, args) {
  return new Promise((resolve, reject) => {
    const id = ++_reqId;
    _pending.set(id, { resolve, reject });
    _benchWorker.postMessage({ id, fn, args });
  });
}

export default function Benchmark() {
  const [data, setData] = useState(null);
  const { baseData, baseMeta, srcData, gndData, geojsonStr } = data || {};
  const [reps, setReps] = useState(2);
  const [running, setRunning] = useState(false);
  const [runs, setRuns] = useState([]);
  const [status, setStatus] = useState('Loading Chudleigh data...');

  useEffect(() => {
    loadExampleData(1000).then(loaded => {
      setData(loaded);
      const { baseData, baseMeta } = loaded;
      const nc = baseData.reduce((c, v) => v !== baseMeta.nodata && v > 0 ? c + 1 : c, 0);
      setStatus(`Loaded Chudleigh 1000×1000`);
    }).catch(err => setStatus(`Error: ${err.message}`));
  }, []);

  const start = useCallback(async () => {
    if (!baseData || running) return;
    setRunning(true); setRuns([]); setStatus('Benchmark running...');

    const nd = baseMeta.nodata || -9999;
    const paramsJson = JSON.stringify(DEFAULT_PARAMS);

    const allResults = [];

    for (const res of RESOLUTIONS) {
      let rsData = baseData, srcR = srcData, gndR = gndData;
      let nrows = res, ncols = res;
      let xmin = baseMeta.xllcorner;
      const ymax = baseMeta.yllcorner + baseMeta.nrows * baseMeta.cellsize;
      let cellsize = baseMeta.cellsize * baseMeta.ncols / res;

      if (res !== baseMeta.nrows || res !== baseMeta.ncols) {
        const ds = (r, nr, nc) => {
          if (nr > baseMeta.nrows || nc > baseMeta.ncols) {
            return upsampleRaster(r, baseMeta.nrows, baseMeta.ncols, nr, nc);
          }
          const json = JSON.parse(downsampleRaster(r, baseMeta.nrows, baseMeta.ncols, nd, nr, nc));
          return new Float64Array(json.data);
        };
        rsData = ds(baseData, res, res);
        srcR = ds(srcData, res, res);
        gndR = ds(gndData, res, res);
      }

      for (let r = 1; r <= reps; r++) {
        for (const runName of RUNS) {
          setStatus(`Running ${res}×${res}, rep ${r}/${reps}: ${runName}...`);
          try {
            const result = await callBenchWorker(`benchmark_${runName}`, [
              rsData, nrows, ncols, nd, geojsonStr, paramsJson, xmin, ymax, cellsize, srcR, gndR, false,
            ]);
            allResults.push({ resolution: res, repeat: r, run: runName, ...result });
            setRuns([...allResults]);
          } catch (err) {
            const msg = (err?.message || String(err)) || 'unknown error';
            allResults.push({ resolution: res, repeat: r, run: runName, prepTimeMs: 0, prepMemMb: 0, connTimeMs: 0, connMemMb: 0, totalIters: 0, error: msg });
            setRuns([...allResults]);
          }
        }
      }
    }

    setStatus(`Run complete: ${allResults.length} runs across ${RESOLUTIONS.length} resolutions`);
    setRunning(false);
  }, [baseData, srcData, gndData, geojsonStr, baseMeta, reps, running]);

  const hasData = !!(baseData && srcData && geojsonStr);

  const fmtMs = (ms) => (ms / 1000).toFixed(3) + 's';
  const fmtMb = (mb) => mb.toFixed(1) + ' MiB';

  const totalRuns = RESOLUTIONS.length * reps * RUNS.length;

  return (
    <div>
      <div className="row">
        <label>repetitions: <input type="number" value={reps} onChange={e => setReps(Math.max(1, Math.min(20, +e.target.value || 1)))}
          disabled={running} /></label>
        <button onClick={start} disabled={!hasData || running}>Run</button>
        {runs.length > 0 && <button onClick={() => buildBenchmarkCSV(runs)}>Download CSV</button>}
      </div>
      <StatusBar status={status} loading={running} />

      {hasData && (
        <div className="bench-info">
        </div>
      )}

      <div className="log">
        <table>
          <thead>
            <tr>
              {BENCHMARK_HEADERS.map((header) => (
                <th key={header}>{header}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {runs.map((r, i) => (
              <tr key={i} className={r.error ? 'error' : ''}>
                <td>{r.resolution}</td><td>{r.repeat}</td>
                <td>{RUN_LABELS[r.run] || r.run}</td>
                <td>{r.error ? 'err' : fmtMs(r.prepTimeMs)}</td>
                <td>{r.error ? 'err' : fmtMb(r.prepMemMb)}</td>
                <td>{r.error ? 'err' : fmtMs(r.connTimeMs)}</td>
                <td>{r.error ? 'err' : fmtMb(r.connMemMb)}</td>
                <td>{r.error ? 'err' : (r.totalIters || 0)}</td>
              </tr>
            ))}
            {running && runs.length < totalRuns && (
              <tr><td colSpan={8}>Running...</td></tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
