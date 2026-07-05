import { useRef, useEffect, useState } from 'react';
import { renderMap, renderPoints } from '../lib/render';

export default function MapView({ type, data, meta, logScale, onToggleScale }) {
  const canvasRef = useRef(null);
  const [range, setRange] = useState({ min: 0, max: 1 });
  const isCurVolt = type === 'cur' || type === 'volt';

  useEffect(() => {
    if (!data || !meta) return;
    const canvas = canvasRef.current;
    if (type === 'points') {
      renderPoints(canvas, data, meta.nrows, meta.ncols, meta.nodata);
    } else {
      const r = renderMap(canvas, data, meta.nrows, meta.ncols, meta.nodata || 0, logScale, 400, isCurVolt);
      if (r) setRange(r);
    }
  }, [data, meta, type, logScale, isCurVolt]);

  const labels = { res:'Resistance', points:'Focal Points', cur:'Current', src:'Source', gnd:'Ground', volt:'Voltage' };
  const dim = meta ? `${meta.ncols}×${meta.nrows}` : '';
  const fmt = (v) => Math.abs(v) < 0.01 ? v.toExponential(1) : Math.abs(v) >= 1000 ? v.toFixed(0) : v.toPrecision(3);

  const legendGrad = isCurVolt
    ? 'linear-gradient(to right,#0d0887,#7e03a8,#cc4778,#f89540,#f0f921)'
    : 'linear-gradient(to right,#00f,#0ff,#0f0,#ff0,#f00)';

  return (
    <div style={{ border: '1px solid #333', display: 'flex', flexDirection: 'column', maxWidth: '100%' }}>
      <div style={{ fontSize: '.65em', padding: '3px 6px', background: '#1a1a1a', borderBottom: '1px solid #333', color: '#888', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <span>{labels[type] || type} {dim && <span style={{ color: '#555' }}>{dim}</span>}</span>
        {onToggleScale && (
          <div style={{ display: 'flex', gap: '2px' }}>
            {['log', 'lin'].map(s => (
              <span key={s} onClick={() => onToggleScale(s)}
                style={{ padding: '1px 4px', border: '1px solid #333', cursor: 'pointer', background: logScale === (s === 'log') ? '#238636' : 'transparent', color: logScale === (s === 'log') ? '#fff' : '#888', fontSize: '.85em' }}>
                {s}
              </span>
            ))}
          </div>
        )}
      </div>
      <canvas ref={canvasRef} style={{ display: 'block', imageRendering: 'pixelated', width: '100%', height: 'auto' }} />
      {type !== 'points' && (
        <div style={{ display: 'flex', gap: '4px', alignItems: 'center', fontSize: '.6em', padding: '2px 6px', color: '#666' }}>
          <span style={{ minWidth: '3.5em', textAlign: 'right' }}>{fmt(range.min)}</span>
          <div style={{ flex: 1, height: '8px', borderRadius: '2px', background: legendGrad }} />
          <span style={{ minWidth: '3.5em' }}>{fmt(range.max)}</span>
        </div>
      )}
    </div>
  );
}
