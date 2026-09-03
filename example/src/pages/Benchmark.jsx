import { useState, useCallback, useEffect } from 'react';
import { downsampleRaster } from '@wasm-connect/lib';
import { Spinner } from '../components/Spinner';
import { StatusBar } from '../components/StatusBar';
import { downloadCSV } from '../stats';
import { loadExampleData } from '../data';

const RESOLUTIONS = [1000, 800, 600, 400, 200];

const DEFAULT_PARAMS = {
  roads:    { resistance: 5,  width: 3 },
  rivers:   { resistance: 0.5, width: 4 },
  buildings:{ resistance: 500, width: 0 },
};

const RUNS = ['jacobi', 'gmg'];

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

function benchmarkJacobi(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, useDirichletGround = false) {
  return callBenchWorker('benchmark_jacobi', [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, useDirichletGround]);
}
function benchmarkGmg(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, useDirichletGround = false) {
  return callBenchWorker('benchmark_gmg', [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, useDirichletGround]);
}

export default function Benchmark() {
  const [baseData, setBaseData] = useState(null);
  const [baseMeta, setBaseMeta] = useState(null);
  const [srcData, setSrcData] = useState(null);
  const [gndData, setGndData] = useState(null);
  const [geojsonStr, setGeojsonStr] = useState('');
  const [reps, setReps] = useState(2);
  const [running, setRunning] = useState(false);
  const [runs, setRuns] = useState([]);
  const [status, setStatus] = useState('Loading Chudleigh data...');

  useEffect(() => {
    loadExampleData(1000).then(({baseData, baseMeta, srcData, gndData, geojsonStr}) => {
      setBaseData(baseData); 
      setBaseMeta(baseMeta);
      setSrcData(srcData);
      setGndData(gndData);
      setGeojsonStr(geojsonStr);
      const nc = baseData.reduce((c, v) => v !== baseMeta.nodata && v > 0 ? c + 1 : c, 0);
      setStatus(`loaded Chudleigh 1000×1000: ${nc.toLocaleString()} conductive cells`);
    });
  }, []);

  const start = useCallback(async () => {
    if (!baseData || running) return;
    setRunning(true); setRuns([]); setStatus('benchmark running...');

    const nd = baseMeta.nodata || -9999;
    const paramsJson = JSON.stringify(DEFAULT_PARAMS);

    const allResults = [];

    for (const res of RESOLUTIONS) {
      let rsData = baseData, srcR = srcData, gndR = gndData;
      let nrows = res, ncols = res;
      let xmin = baseMeta.xllcorner;
      const ymax = baseMeta.yllcorner + baseMeta.nrows * baseMeta.cellsize;
      let cellsize = 1000 / res;

      if (res !== 1000) {
        const ds = (r, nr, nc) => {
          const json = JSON.parse(downsampleRaster(r, 1000, 1000, nd, nr, nc));
          return new Float64Array(json.data);
        };
        rsData = ds(baseData, res, res);
        srcR = ds(srcData, res, res);
        gndR = ds(gndData, res, res);
      }

      for (let r = 1; r <= reps; r++) {
        for (const runName of RUNS) {
          setStatus(`running ${res}×${res}, rep ${r}/${reps}: ${runName}...`);
          try {
            let result;
            if (runName === 'jacobi') {
              result = await benchmarkJacobi(rsData, nrows, ncols, nd, geojsonStr, paramsJson, xmin, ymax, cellsize, srcR, gndR);
            } else {
              result = await benchmarkGmg(rsData, nrows, ncols, nd, geojsonStr, paramsJson, xmin, ymax, cellsize, srcR, gndR);
            }
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

    setStatus(`done — ${allResults.length} runs across ${RESOLUTIONS.length} resolutions`);
    setRunning(false);
  }, [baseData, srcData, gndData, geojsonStr, baseMeta, reps, running]);

  const hasData = !!(baseData && srcData && geojsonStr);

  const fmtMs = (ms) => (ms / 1000).toFixed(3) + 's';
  const fmtMb = (mb) => mb.toFixed(1) + ' MB';

  const totalRuns = RESOLUTIONS.length * reps * RUNS.length;

  return (
    <div>
      <div className="row">
        <span className="bench-reps-label">repetitions:</span>
        <input type="number" value={reps} onChange={e => setReps(Math.max(1, Math.min(20, +e.target.value || 1)))}
          disabled={running} className="bench-input" />
        <button className="btn run" onClick={start} disabled={!hasData || running}>Run</button>
        {runs.length > 0 && <button className="btn" onClick={() => buildBenchmarkCSV(runs)}>Download CSV</button>}
      </div>
      <StatusBar status={status} loading={running} />

      {hasData && (
        <div className="bench-info">
          Chudleigh 1000×1000 · resolutions: {RESOLUTIONS.join(', ')} · {reps} rep{reps !== 1 ? 's' : ''} · 2 solvers per rep: Jacobi CG, GMG CG
        </div>
      )}

      <div className="log log--bench">
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
              <tr key={i} className={r.error ? 'bench-log-row--error' : ''}>
                <td>{r.resolution}</td><td>{r.repeat}</td>
                <td>{r.run}</td>
                <td>{r.error ? 'err' : fmtMs(r.prepTimeMs)}</td>
                <td>{r.error ? 'err' : fmtMb(r.prepMemMb)}</td>
                <td>{r.error ? 'err' : fmtMs(r.connTimeMs)}</td>
                <td>{r.error ? 'err' : fmtMb(r.connMemMb)}</td>
                <td>{r.error ? 'err' : (r.totalIters || 0)}</td>
              </tr>
            ))}
            {running && runs.length < totalRuns && (
              <tr><td colSpan={8}><Spinner size={12} /> running...</td></tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
