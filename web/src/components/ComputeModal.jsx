import { Spinner } from './Spinner';

export function ComputeModal({ visible }) {
  if (!visible) return null;
  return (
    <div style={{ position: 'fixed', inset: 0, zIndex: 100, display: 'flex', alignItems: 'center', justifyContent: 'center', background: 'rgba(0,0,0,0.75)' }}
      onClick={e => e.stopPropagation()}>
      <div style={{ textAlign: 'center', padding: '28px 40px', background: '#1a1a1a', border: '1px solid #444', borderRadius: 6 }}>
        <Spinner size={28} />
        <div style={{ marginTop: 14, fontSize: '.85em' }}>Computing...</div>
      </div>
    </div>
  );
}
