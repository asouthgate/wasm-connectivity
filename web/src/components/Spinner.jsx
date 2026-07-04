export function Spinner({ size = 14 }) {
  return (
    <span style={{
      display: 'inline-block', width: size, height: size,
      border: '2px solid #333', borderTopColor: '#58a6ff',
      borderRadius: '50%', animation: 'spin .8s linear infinite',
      verticalAlign: 'middle', marginRight: 6, flexShrink: 0,
    }} />
  );
}
