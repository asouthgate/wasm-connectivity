import { Spinner } from './Spinner';

export function ComputeModal({ visible }) {
  if (!visible) return false;
  return (
    <div className="modal-overlay" onClick={e => e.stopPropagation()}>
      <div className="modal-box">
        <Spinner size={28} />
        <div className="modal-status">Computing...</div>
      </div>
    </div>
  );
}
