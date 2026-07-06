import { useState } from 'react';

export function useStatus(initialText, initialColor = '#888') {
  return useState({ text: initialText, color: initialColor });
}

export function useLoadingTimer() {
  const [loading, setLoading] = useState(false);
  const [timer, setTimer] = useState('');
  return { loading, setLoading, timer, setTimer };
}
