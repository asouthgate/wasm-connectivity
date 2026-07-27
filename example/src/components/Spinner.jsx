export function Spinner({ size = 14 }) {
  return (
    <span className="spinner" style={{ '--spinner-size': size + 'px' }} />
  );
}
