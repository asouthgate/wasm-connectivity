import { useEffect, useRef, useState } from 'react';
import MapView from '../components/MapView';
import { StatusBar } from '../components/StatusBar';
import { SCENARIOS } from '../scenarios';

export default function ScenarioBenchmark() {
  const [previews, setPreviews] = useState({});
  const [results, setResults] = useState({});
  const [precision, setPrecision] = useState('f64');
  const [running, setRunning] = useState(false);
  const [status, setStatus] = useState('Loading scenario previews...');
  const [error, setError] = useState(false);
  const active = useRef(null);
  const mounted = useRef(false);
  const busy = useRef(false);

  useEffect(() => {
    mounted.current = true;
    const controller = new AbortController();
    Promise.all(SCENARIOS.map(async ({ id }) => {
      const response = await fetch(`/geodata/scenarios/${id}_preview.json`, { signal: controller.signal });
      if (!response.ok) throw new Error(`Could not load ${id} preview`);
      return [id, await response.json()];
    })).then(entries => {
      if (!mounted.current) return;
      setPreviews(Object.fromEntries(entries));
      setStatus('2000x2000 - low-memory MG - Shunt ground');
    }).catch(err => {
      if (err.name === 'AbortError' || !mounted.current) return;
      setStatus(err.message); setError(true);
    });
    return () => {
      mounted.current = false;
      controller.abort();
      active.current?.cancel();
    };
  }, []);

  async function run() {
    if (busy.current) return;
    busy.current = true;
    setRunning(true); setResults({}); setError(false);
    let failed = 0;
    try {
      for (const scenario of SCENARIOS) {
        if (!mounted.current) break;
        setStatus(`Running ${scenario.name} (${precision})...`);
        const worker = new Worker(new URL('../workers/benchmark.js', import.meta.url), { type: 'module' });
        try {
          const result = await new Promise((resolve, reject) => {
            active.current = { cancel: () => { worker.terminate(); reject(new Error('Run cancelled')); } };
            worker.onmessage = ({ data }) => data.error ? reject(new Error(data.error)) : resolve(data.result);
            worker.onerror = event => reject(new Error(event.message || 'Worker failed'));
            worker.postMessage({ id: 1, fn: 'benchmark_scenario', args: [scenario.id, precision] });
          });
          if (mounted.current) setResults(previous => ({ ...previous, [scenario.id]: result }));
        } catch (err) {
          failed++;
          if (mounted.current) setResults(previous => ({ ...previous, [scenario.id]: { error: err.message } }));
        } finally {
          worker.terminate();
          active.current = null;
        }
      }
    } finally {
      busy.current = false;
      if (mounted.current) {
        setRunning(false); setError(failed > 0);
        setStatus(failed ? `Finished · ${failed} scenario${failed === 1 ? '' : 's'} failed` : `Finished · 3 scenarios · ${precision}`);
      }
    }
  }

  return (
    <div>
      <div className="row">
        <label>Precision
          <select value={precision} disabled={running} onChange={e => {
            setPrecision(e.target.value); setResults({}); setError(false);
            setStatus(`2000x2000 · low-memory MG · Shunt ground · ${e.target.value}`);
          }}>
            <option value="f64">f64</option><option value="f32">f32</option><option value="mixed">Mixed precision</option>
          </select>
        </label>
        <button onClick={run} disabled={running || Object.keys(previews).length !== 3}>Run</button>
      </div>
      <StatusBar status={status} loading={running} error={error} />
      {SCENARIOS.map(scenario => {
        const preview = previews[scenario.id];
        const result = results[scenario.id];
        return (
          <section key={scenario.id} className="scenario">
            {preview && <div className="scenario-previews">
              {['res', 'src', 'gnd'].map(type => <div key={type} className="scenario-preview">
                <MapView type={type}
                  image={preview[type]} meta={preview.meta} preview maxSide={200} showHeader={false}
                  quantileScale={scenario.id === 'point_lights' && type === 'res'} />
                {scenario.id === 'point_lights' && type === 'res' &&
                  <span className="scenario-preview-asterisk" title="Quantile-stretched resistance (500 classes)">*</span>}
              </div>)}
              {result?.currentMap ? <div className="scenario-preview">
                <MapView type="cur" label="Current"
                  data={result.currentMap.data} meta={result.currentMap} preview maxSide={200} showHeader={false}
                  quantileScale />
                <span className="scenario-preview-asterisk" title="Quantile-stretched current (500 classes)">*</span>
              </div> :
                <div className="map-view">
                  <div className="scenario-current-pending" />
                </div>}
            </div>}
          </section>
        );
      })}
      <div className="scenario">
        <div className="bench-info">
          Previews: 200x200 · columns: resistance, source, ground, current.
          Full-resolution current maps: 2000x2000, in the same scenario order. * Quantile stretch (500 classes).
        </div>
        {SCENARIOS.map((scenario, index) => {
          const result = results[scenario.id];
          return <section key={scenario.id} className="scenario">
            <h3>{index + 1}. {scenario.name}</h3>
            <div className="bench-info">{scenario.description}</div>
            {result && <div className="log">
              {result.error ? <div className="error">{result.error}</div> : <table>
                <thead><tr><th>Runtime (s)</th><th>Memory (MiB)</th><th>Iterations</th></tr></thead>
                <tbody><tr><td>{(result.connTimeMs / 1000).toFixed(3)}</td><td>{result.connMemMb.toFixed(1)}</td><td>{result.totalIters}</td></tr></tbody>
              </table>}
            </div>}
          </section>;
        })}
      </div>
      {SCENARIOS.map(scenario => {
        const current = results[scenario.id]?.currentMap;
        return current && <section key={scenario.id} className="scenario">
          <div className="scenario-current scenario-preview">
            <MapView type="cur" label="Current" data={current.data} meta={current} showHeader={false}
              quantileScale />
            <span className="scenario-preview-asterisk" title="Quantile-stretched current (500 classes)">*</span>
          </div>
        </section>;
      })}
    </div>
  );
}
