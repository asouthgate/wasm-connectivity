import { useState, useCallback, useEffect } from 'react';
import { benchmarkCold, benchmarkWarm, benchmarkHot, run_geospatial_pipeline_cached_async, downsample_raster, reset_cache_async } from '../lib/wasm';
import { parseAsc } from '../lib/parseAsc';
import { Spinner } from '../components/Spinner';
import { StatusBar } from '../components/StatusBar';
import { stat, downloadCSV } from '../lib/stats';
import { useStatus } from '../lib/hooks';
import { randomSquareBuildings, mergeGeoJson } from '../lib/randomBuildings';

const GEO_BASE = '/geodata';
const RESOLUTIONS = [1000, 800, 600, 400, 200];

const DEFAULT_PARAMS = {
  roads:    { resistance: 50,  width: 3 },
  rivers:   { resistance: 0.5, width: 4 },
  buildings:{ resistance: 500, width: 0 },
};

const PHASES = ['cold', 'warm', 'hot'];

function perturbSources(srcData, ncols, nrows, nodata) {
  // Perturb ~5% of source cells by a small random factor; produce a
  // fresh copy so the warm phase is not returning the exact same RHS as
  // the cold solve (which would just give back the identical voltage
  // solution in trivial CG iteration count).
  const out = Float64Array.from(srcData);
  const n = ncols * nrows;
  const count = Math.max(1, Math.floor(n * 0.005));
  let s = 0xabcdef12 >>> 0;
  const rnd = () => {
    s = (s + 0x6D2B79F5) | 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  for (let i = 0; i < count; i++) {
    const idx = Math.floor(rnd() * n);
    const v = out[idx];
    if (v === nodata || !isFinite(v) || v <= 0) continue;
    const factor = 0.5 + rnd() * 1.5; // [0.5, 2.0]
    out[idx] = v * factor;
  }
  return out;
}

function buildBenchmarkCSV(runs) {
  const h = 'resolution,repeat,phase,prep_time_s,prep_mem_mb,conn_time_s,conn_mem_mb,total_iters\n';
  const rows = runs.map(r =>
    `${r.resolution},${r.repeat},${r.phase},${(r.prepTimeMs/1000).toFixed(4)},${r.prepMemMb.toFixed(2)},${(r.connTimeMs/1000).toFixed(4)},${r.connMemMb.toFixed(2)},${r.totalIters||0}`
  ).join('\n');
  downloadCSV('benchmark.csv', h, rows);
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
    let warmSeedCounter = 0;

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
        // === Phase 1: COLD ===
        setStatus({ text: `Running ${res}×${res}, rep ${r}/${reps}: cold...`, color: '#f90' });
        try {
          // Ensure no stale cache before the cold call (it starts from
          // __reset internally too, but be explicit).
          await reset_cache_async();
          const cold = await benchmarkCold(rsData, nrows, ncols, nd, geojsonStr, paramsJson, xmin, ymax, cellsize, srcR, gndR);
          allResults.push({ resolution: res, repeat: r, phase: 'cold', ...cold });
          setRuns([...allResults]);

          // === Phase 2: WARM (perturbed sources, no rebuild) ===
          setStatus({ text: `Running ${res}×${res}, rep ${r}/${reps}: warm...`, color: '#f90' });
          const srcPert = perturbSources(srcR, ncols, nrows, nd);
          const warm = await benchmarkWarm(srcPert, gndR, nrows, ncols, nd);
          allResults.push({ resolution: res, repeat: r, phase: 'warm', ...warm });
          setRuns([...allResults]);

          // === Phase 3: HOT (3 random building squares, rebuild=true) ===
          setStatus({ text: `Running ${res}×${res}, rep ${r}/${reps}: hot...`, color: '#f90' });
          const seed = (Date.now() + (++warmSeedCounter) * 7919) & 0xffffffff;
          const extra = randomSquareBuildings(3, nrows, ncols, xmin, ymax, cellsize, 0.10, seed);
          const hotGeo = mergeGeoJson(geojsonStr, extra);
          const hot = await benchmarkHot(rsData, nrows, ncols, nd, hotGeo, paramsJson, xmin, ymax, cellsize, srcR, gndR);
          allResults.push({ resolution: res, repeat: r, phase: 'hot', ...hot });
          setRuns([...allResults]);
        } catch (err) {
          allResults.push({ resolution: res, repeat: r, phase: '?', prepTimeMs: 0, prepMemMb: 0, connTimeMs: 0, connMemMb: 0, totalIters: 0, error: err.message });
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

  // Summary per (resolution, phase)
  const summary = {};
  for (const r of runs) {
    if (r.error) continue;
    const key = `${r.resolution}_${r.phase}`;
    if (!summary[key]) summary[key] = { prepTimes: [], connTimes: [], prepMems: [], connMems: [], iters: [] };
    summary[key].prepTimes.push(r.prepTimeMs / 1000);
    summary[key].connTimes.push(r.connTimeMs / 1000);
    summary[key].prepMems.push(r.prepMemMb);
    summary[key].connMems.push(r.connMemMb);
    summary[key].iters.push(r.totalIters || 0);
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
          Chudleigh 1000×1000 · resolutions: {RESOLUTIONS.join(', ')} · {reps} rep{reps !== 1 ? 's' : ''} · 3 phases per rep: <span style={{color:'#58a6ff'}}>cold</span> · <span style={{color:'#f08080'}}>warm</span> · <span style={{color:'#f90'}}>hot</span>
          <div style={{ marginTop: 4, fontSize: '.95em' }}>
            <span style={{color:'#58a6ff'}}>cold</span> = full WASM reset+rasterize+cold solve (caches the circuit model)  ·
            <span style={{color:'#f08080'}}> warm</span> = same resistance + perturbed sources, cache reuse (no rebuild)  ·
            <span style={{color:'#f90'}}> hot</span> = 3 random square buildings added, rebuild laplacian, PCG seeded from prior voltage
          </div>
        </div>
      )}

      <div className="log" style={{ maxHeight: '50vh' }}>
        <table>
          <thead><tr>
            <th>res</th><th>rep</th><th>phase</th><th>prep time</th><th>prep mem</th><th>conn time</th><th>conn mem</th><th>iters</th>
          </tr></thead>
          <tbody>
            {runs.map((r, i) => (
              <tr key={i} style={r.error ? { color: '#f44' } : {}}>
                <td>{r.resolution}</td><td>{r.repeat}/{reps}</td>
                <td style={{ color: r.phase === 'cold' ? '#58a6ff' : r.phase === 'warm' ? '#f08080' : r.phase === 'hot' ? '#f90' : '#f44' }}>{r.phase}</td>
                <td>{r.error ? 'err' : fmtMs(r.prepTimeMs)}</td>
                <td>{r.error ? 'err' : fmtMb(r.prepMemMb)}</td>
                <td>{r.error ? 'err' : fmtMs(r.connTimeMs)}</td>
                <td>{r.error ? 'err' : fmtMb(r.connMemMb)}</td>
                <td>{r.error ? 'err' : (r.totalIters || 0)}</td>
              </tr>
            ))}
            {running && runs.length < RESOLUTIONS.length * reps * 3 && (
              <tr><td colSpan={8} style={{ color: '#f90' }}><Spinner size={12} /> running...</td></tr>
            )}
          </tbody>
        </table>
      </div>

      {Object.keys(summary).length > 0 && (
        <div style={{ marginTop: 12 }}>
          <h3 style={{ marginBottom: 4 }}>Summary (μ over reps)</h3>
          <div className="log" style={{ maxHeight: '50vh' }}>
            <table>
              <thead><tr><th>res</th><th>phase</th><th>n</th><th>prep μ</th><th>prep mem μ</th><th>conn μ</th><th>conn σ</th><th>conn mem μ</th><th>iters μ</th></tr></thead>
              <tbody>
                {RESOLUTIONS.flatMap(res =>
                  PHASES.map(phase => {
                    const key = `${res}_${phase}`;
                    const s = summary[key];
                    if (!s) return null;
                    const pt = stat(s.prepTimes), ct = stat(s.connTimes), pm = stat(s.prepMems), cm = stat(s.connMems), it = stat(s.iters);
                    return (
                      <tr key={key}>
                        <td>{res}</td>
                        <td style={{ color: phase === 'cold' ? '#58a6ff' : phase === 'warm' ? '#f08080' : '#f90' }}>{phase}</td>
                        <td>{s.connTimes.length}</td>
                        <td>{pt.mean?.toFixed(3)}s</td>
                        <td>{pm.mean?.toFixed(1)} MB</td>
                        <td>{ct.mean?.toFixed(3)}s</td>
                        <td>{ct.stddev?.toFixed(3)}s</td>
                        <td>{cm.mean?.toFixed(1)} MB</td>
                        <td>{it.mean?.toFixed(0)}</td>
                      </tr>
                    );
                  })
                )}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  );
}