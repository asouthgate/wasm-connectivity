import { useState, useCallback, useEffect } from 'react';
import { runBenchmark, downsample_raster } from '../lib/wasm';
import { parseAsc } from '../lib/parseAsc';
import { Spinner } from '../components/Spinner';
import { StatusBar } from '../components/StatusBar';
import { stat, downloadCSV } from '../lib/stats';
import { useStatus } from '../lib/hooks';

const GEO_BASE = '/geodata';
const RESOLUTIONS = [1000, 800, 600, 400, 200];

const DEFAULT_PARAMS = {
  roads:    { resistance: 50,  width: 3 },
  rivers:   { resistance: 0.5, width: 4 },
  buildings:{ resistance: 500, width: 0 },
};

function buildBenchmarkCSV(runs) {
  const h = 'resolution,repeat,prep_time_s,prep_mem_mb,conn_time_s,conn_mem_mb\n';
  const rows = runs.map(r => `${r.resolution},${r.repeat},${(r.prepTimeMs/1000).toFixed(4)},${r.prepMemMb.toFixed(2)},${(r.connTimeMs/1000).toFixed(4)},${r.connMemMb.toFixed(2)}`).join('\n');
  downloadCSV('benchmark.csv', h, rows);
}

export default function Benchmark() {
  const [baseData, setBaseData] = useState(null);
  const [baseMeta, setBaseMeta] = useState(null);
  const [srcData, setSrcData] = useState(null);
  const [gndData, setGndData] = useState(null);
  const [geojsonStr, setGeojsonStr] = useState('');
  const [reps, setReps] = useState(3);
  const [running, setRunning] = useState(false);
  const [runs, setRuns] = useState([]);
  const [status, setStatus] = useState({ text: 'Loading Chudleigh data...', color: '#888' });

  useEffect(() => {
    (async () => {
      const [baseText, srcText, gndText, geoResp] = await Promise.all([
        fetch(`${GEO_BASE}/base_resistance_1000.asc`).then(r => r.text()),
        fetch(`${GEO_BASE}/source_1000.asc`).then(r => r.text()),
        fetch(`${GEO_BASE}/ground_1000.asc`).then(r => r.text()),
        fetch(`${GEO_BASE}/all_features_1000.geojson`).then(r => r.text()),
      ]);
      const bp = parseAsc(baseText);
      setBaseData(bp.data); setBaseMeta(bp.meta);
      setSrcData(parseAsc(srcText).data);
      setGndData(parseAsc(gndText).data);
      setGeojsonStr(geoResp);
      const nc = bp.data.reduce((c, v) => v !== bp.meta.nodata && v > 0 ? c + 1 : c, 0);
      setStatus({ text: `loaded Chudleigh 1000×1000 — ${nc.toLocaleString()} conductive cells`, color: '#58a6ff' });
    })();
  }, []);

  const start = useCallback(async () => {
    if (!baseData || running) return;
    setRunning(true); setRuns([]); setStatus({ text: 'benchmark running...', color: '#f90' });

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
          const json = JSON.parse(downsample_raster(r, 1000, 1000, nd, nr, nc));
          return new Float64Array(json.data);
        };
        rsData = ds(baseData, res, res);
        srcR = ds(srcData, res, res);
        gndR = ds(gndData, res, res);
      }

      for (let r = 1; r <= reps; r++) {
        setStatus({ text: `Running ${res}×${res}, repeat ${r}/${reps}...`, color: '#f90' });
        try {
          const bench = await runBenchmark(rsData, nrows, ncols, nd, geojsonStr, paramsJson, xmin, ymax, cellsize, srcR, gndR);
          allResults.push({
            resolution: res,
            repeat: r,
            prepTimeMs: bench.prepTimeMs,
            prepMemMb: bench.prepMemMb,
            connTimeMs: bench.connTimeMs,
            connMemMb: bench.connMemMb,
          });
          setRuns([...allResults]);
        } catch (err) {
          allResults.push({ resolution: res, repeat: r, prepTimeMs: 0, prepMemMb: 0, connTimeMs: 0, connMemMb: 0, error: err.message });
          setRuns([...allResults]);
        }
      }
    }

    setStatus({ text: `done — ${allResults.length} runs across ${RESOLUTIONS.length} resolutions`, color: '#3fb950' });
    setRunning(false);
  }, [baseData, srcData, gndData, geojsonStr, baseMeta, reps, running]);

  const hasData = !!(baseData && srcData && geojsonStr);

  const fmtMs = (ms) => (ms / 1000).toFixed(3) + 's';
  const fmtMb = (mb) => mb.toFixed(1) + ' MB';

  const summaryByRes = {};
  for (const r of runs) {
    if (r.error) continue;
    const key = r.resolution;
    if (!summaryByRes[key]) summaryByRes[key] = { prepTimes: [], connTimes: [], prepMems: [], connMems: [] };
    summaryByRes[key].prepTimes.push(r.prepTimeMs / 1000);
    summaryByRes[key].connTimes.push(r.connTimeMs / 1000);
    summaryByRes[key].prepMems.push(r.prepMemMb);
    summaryByRes[key].connMems.push(r.connMemMb);
  }

  return (
    <div>
      <div className="row">
        <span style={{ fontSize: '.75em', color: '#888' }}>repetitions:</span>
        <input type="number" value={reps} onChange={e => setReps(Math.max(1, Math.min(20, +e.target.value || 1)))}
          disabled={running} style={{ width: 50, background: '#222', border: '1px solid #444', color: '#ccc', padding: '3px 6px', font: '12px monospace' }} />
        <button className="btn run" onClick={start} disabled={!hasData || running}>Run</button>
        {runs.length > 0 && <button className="btn" onClick={() => buildBenchmarkCSV(runs)}>Download CSV</button>}
      </div>
      <StatusBar status={status} loading={running} />

      {hasData && (
        <div style={{ fontSize: '.75em', color: '#888', marginBottom: 8 }}>
          Chudleigh 1000×1000 · resolutions: {RESOLUTIONS.join(', ')} · {reps} repeat{reps !== 1 ? 's' : ''}
        </div>
      )}

      <div className="log" style={{ maxHeight: '60vh' }}>
        <table>
          <thead><tr>
            <th>res</th><th>rep</th><th>prep time</th><th>prep mem</th><th>conn time</th><th>conn mem</th>
          </tr></thead>
          <tbody>
            {runs.map((r, i) => (
              <tr key={i} style={r.error ? { color: '#f44' } : {}}>
                <td>{r.resolution}</td><td>{r.repeat}/{reps}</td>
                <td>{r.error ? 'err' : fmtMs(r.prepTimeMs)}</td>
                <td>{r.error ? 'err' : fmtMb(r.prepMemMb)}</td>
                <td>{r.error ? 'err' : fmtMs(r.connTimeMs)}</td>
                <td>{r.error ? 'err' : fmtMb(r.connMemMb)}</td>
              </tr>
            ))}
            {running && runs.length < RESOLUTIONS.length * reps && (
              <tr><td colSpan={6} style={{ color: '#f90' }}><Spinner size={12} /> running...</td></tr>
            )}
          </tbody>
        </table>
      </div>

      {Object.keys(summaryByRes).length > 0 && (
        <div style={{ marginTop: 12 }}>
          <h3 style={{ marginBottom: 4 }}>Summary</h3>
          <div className="log" style={{ maxHeight: '50vh' }}>
            <table>
              <thead><tr><th>res</th><th>n</th><th>prep μ</th><th>prep σ</th><th>prep mem μ</th><th>conn μ</th><th>conn σ</th><th>conn mem μ</th></tr></thead>
              <tbody>
                {RESOLUTIONS.filter(r => summaryByRes[r]).map(res => {
                  const s = summaryByRes[res];
                  const pt = stat(s.prepTimes), ct = stat(s.connTimes), pm = stat(s.prepMems), cm = stat(s.connMems);
                  return (
                    <tr key={res}>
                      <td>{res}</td><td>{s.prepTimes.length}</td>
                      <td>{pt.mean?.toFixed(3)}s</td><td>{pt.stddev?.toFixed(3)}s</td>
                      <td>{pm.mean?.toFixed(1)} MB</td>
                      <td>{ct.mean?.toFixed(3)}s</td><td>{ct.stddev?.toFixed(3)}s</td>
                      <td>{cm.mean?.toFixed(1)} MB</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  );
}
