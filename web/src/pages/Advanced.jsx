import { useState, useCallback } from 'react';
import { flushSync } from 'react-dom';
import { solve_advanced } from '../lib/wasm';
import { parseAsc } from '../lib/parseAsc';
import { PRESETS } from '../lib/presets';
import MapView from '../components/MapView';
import { Spinner } from '../components/Spinner';

const PAIRWISE = PRESETS.filter(g => g.mode === 'pairwise');
const ADVANCED = PRESETS.filter(g => g.mode === 'advanced');

export default function Advanced() {
  const [inputMode, setInputMode] = useState('raster');
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
  const [resName, setResName] = useState('—');
  const [s1Name, setS1Name] = useState('—');
  const [s2Name, setS2Name] = useState('—');

  const presets = inputMode === 'raster' ? ADVANCED : PAIRWISE;

  const loadPreset = useCallback(async (p) => {
    setStatus({ text: `loading ${p.name}...`, color: '#888' });
    setSelPreset(p.id); setResult(null); setTimer('');

    if (inputMode === 'raster') {
      const [rt, st, gt] = await Promise.all([
        fetch(p.res).then(r=>r.text()), fetch(p.src).then(r=>r.text()), fetch(p.gnd).then(r=>r.text())
      ]);
      const rp = parseAsc(rt), sp = parseAsc(st), gp = parseAsc(gt);
      setResData(rp.data); setResMeta(rp.meta);
      setSrcData(sp.data); setSrcMeta(sp.meta);
      setGndData(gp.data); setGndMeta(gp.meta);
      setResName(p.res.split('/').pop()); setS1Name(p.src.split('/').pop()); setS2Name(p.gnd.split('/').pop());
    } else {
      const [rt, pt] = await Promise.all([fetch(p.res).then(r=>r.text()), fetch(p.pts).then(r=>r.text())]);
      const rp = parseAsc(rt), pp = parseAsc(pt);
      const n = rp.meta.nrows * rp.meta.ncols;
      const nd = pp.meta.nodata;
      const srcR = new Float64Array(n), gndR = new Float64Array(n);
      for (let i = 0; i < n; i++) {
        const v = pp.data[i];
        if (v === nd || isNaN(v) || v <= 0) continue;
        if (v === 1) srcR[i] = 1.0;
        else gndR[i] = 1.0;
      }
      setResData(rp.data); setResMeta(rp.meta);
      setSrcData(srcR); setSrcMeta({ nrows: rp.meta.nrows, ncols: rp.meta.ncols, nodata: -9999 });
      setGndData(gndR); setGndMeta({ nrows: rp.meta.nrows, ncols: rp.meta.ncols, nodata: -9999 });
      setResName(p.res.split('/').pop()); setS1Name(p.pts.split('/').pop()); setS2Name(p.pts.split('/').pop());
      const nc = rp.data.reduce((c,v)=>v!==rp.meta.nodata&&v>0?c+1:c,0);
      setStatus({ text: `loaded ${p.name} — id=1→src, id>1→sink`, color: '#58a6ff' });
      return;
    }

    const nc = rp.data.reduce((c,v)=>v!==rp.meta.nodata&&v>0?c+1:c,0);
    setStatus({ text: `loaded ${p.name} — ${nc.toLocaleString()} cells`, color: '#58a6ff' });
  }, [inputMode]);

  const handleFile = useCallback((setter, setMeta, setName) => async (e) => {
    const f = e.target.files[0]; if (!f) return;
    setSelPreset(null); setName(f.name);
    const t = await f.text(); const p = parseAsc(t);
    setter(p.data); setMeta(p.meta);
  }, []);

  const run = useCallback(async () => {
    if (!resData||!srcData) return;
    flushSync(() => {
      setLoading(true); setStatus({ text: 'solving...', color: '#f90' }); setTimer('');
    });
    await new Promise(r => requestAnimationFrame(r));
    const t0 = performance.now();
    try {
      const nd = resMeta.nodata||-9999;
      const gd = gndData || new Float64Array(resMeta.nrows * resMeta.ncols);
      const r = JSON.parse(solve_advanced(resData, resMeta.nrows, resMeta.ncols, nd, srcData, gd));
      setResult(r);
      const s = ((performance.now()-t0)/1000).toFixed(1);
      setTimer(`${s}s`);
      setStatus({ text: `solved in ${s}s`, color: '#3fb950' });
    } catch (err) {
      setStatus({ text: `error: ${err.message}`, color: '#f44' });
    } finally {
      setLoading(false);
    }
  }, [resData, srcData, gndData, resMeta]);

  const hasData = !!(resData && srcData);

  return (
    <div>
      <div className="row">
        <button className="btn run" onClick={run} disabled={!hasData || loading}>Run</button>
        <span className="timer">{timer}</span>
        <span style={{ fontSize: '.75em', color: '#888' }}>input:</span>
        <span className={'preset'+(inputMode==='raster'?' sel':'')} onClick={()=>!loading&&setInputMode('raster')}
          style={{cursor:loading?'not-allowed':'pointer',opacity:loading?0.5:1}}>rasters</span>
        <span className={'preset'+(inputMode==='points'?' sel':'')} onClick={()=>!loading&&setInputMode('points')}
          style={{cursor:loading?'not-allowed':'pointer',opacity:loading?0.5:1}}>points</span>
        <span style={{ fontSize: '.75em', color: '#555' }}>custom:</span>
        <label className="btn" htmlFor="advResFile" style={loading?{pointerEvents:'none',opacity:.5}:{}}>+ res</label>
        <input type="file" id="advResFile" accept=".asc,.txt" onChange={handleFile(setResData, setResMeta, setResName)} disabled={loading} style={{display:'none'}} />
        <span className="fname">{resName}</span>
        <label className="btn" htmlFor="advS1File" style={loading?{pointerEvents:'none',opacity:.5}:{}}>+ {inputMode==='raster'?'src':'src pts'}</label>
        <input type="file" id="advS1File" accept=".asc,.txt" onChange={handleFile(setSrcData, setSrcMeta, setS1Name)} disabled={loading} style={{display:'none'}} />
        <span className="fname">{s1Name}</span>
        <label className="btn" htmlFor="advGndFile" style={loading?{pointerEvents:'none',opacity:.5}:{}}>+ {inputMode==='raster'?'gnd':'sink pts'}</label>
        <input type="file" id="advGndFile" accept=".asc,.txt" onChange={handleFile(setGndData, setGndMeta, setS2Name)} disabled={loading} style={{display:'none'}} />
        <span className="fname">{s2Name}</span>
      </div>
      <div className="status" style={{ color: status.color, display: 'flex', alignItems: 'center', position: 'relative', zIndex: 91 }}>
        {loading && <Spinner />}{status.text}
      </div>

      <div className="layout">
        <div className="sidebar">
          {presets.map(grp => (
            <div key={grp.group} className="preset-group">
              <h3>{grp.group}</h3>
              {grp.items.map(p => (
                <div key={p.id} className={'preset'+(selPreset===p.id?' sel':'')}
                  onClick={()=>!loading&&loadPreset(p)} style={loading?{pointerEvents:'none',opacity:.5}:{}}>{p.name}</div>
              ))}
            </div>
          ))}
        </div>
        <div className="main">
          {hasData && (
            <div className="maps" style={{ gridTemplateColumns: 'repeat(3, 1fr)' }}>
              <MapView type="res" data={resData} meta={resMeta} logScale={resScale==='log'} onToggleScale={setResScale} />
              <MapView type="src" data={srcData} meta={{...srcMeta||resMeta, nodata: resMeta.nodata}} logScale={false} />
              {gndData && <MapView type="gnd" data={gndData} meta={{...gndMeta||resMeta, nodata: resMeta.nodata}} logScale={false} />}
            </div>
          )}
          {result && (
            <div className="maps" style={{ gridTemplateColumns: 'repeat(2, 1fr)', marginTop: 8 }}>
              <MapView type="volt" data={result.voltages} meta={{ nrows:result.nrows, ncols:result.ncols, nodata:0 }} logScale={voltScale==='log'} onToggleScale={setVoltScale} />
              <MapView type="cur" data={result.current_map} meta={{ nrows:result.nrows, ncols:result.ncols, nodata:0 }} logScale={curScale==='log'} onToggleScale={setCurScale} />
            </div>
          )}
        </div>
      </div>
      {loading && <div style={{ position: 'fixed', inset: 0, zIndex: 90, cursor: 'not-allowed' }} onClick={e => e.stopPropagation()} />}
    </div>
  );
}
