export default function LoadingModal({ show }) {
  if (!show) return null;
  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.75)', zIndex: 100, display: 'flex', justifyContent: 'center', alignItems: 'center', flexDirection: 'column' }}>
      <div style={{ width: 32, height: 32, border: '3px solid #333', borderTopColor: '#58a6ff', borderRadius: '50%', animation: 'spin .8s linear infinite', marginBottom: 10 }} />
      <div style={{ color: '#888', fontSize: '.85em' }}>Solving...</div>
    </div>
  );
}
