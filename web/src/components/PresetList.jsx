export function PresetList({ presets, selectedId, onSelect, disabled }) {
  return (
    <>
      {presets.map(grp => (
        <div key={grp.group} className="preset-group">
          <h3>{grp.group}</h3>
          {grp.items.map(p => (
            <div key={p.id} className={'preset' + (selectedId === p.id ? ' sel' : '')}
              onClick={() => !disabled && onSelect(p)}
              style={disabled ? { pointerEvents: 'none', opacity: 0.5 } : {}}>{p.name}</div>
          ))}
        </div>
      ))}
    </>
  );
}
