import { useEffect, useRef, useState, useCallback } from 'react';

export function useAutoScroll(deps: unknown[]) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const endRef = useRef<HTMLDivElement | null>(null);
  const [shouldAutoScroll, setShouldAutoScroll] = useState(true);
  const isFirstLoad = useRef(true);

  const handleScroll = useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    const { scrollHeight, scrollTop, clientHeight } = el;
    const isNearBottom = scrollHeight - scrollTop - clientHeight < 100;
    setShouldAutoScroll(isNearBottom);
  }, []);

  const scrollToBottom = useCallback(() => {
    endRef.current?.scrollIntoView({ behavior: 'smooth' });
    setShouldAutoScroll(true);
  }, []);

  useEffect(() => {
    if (isFirstLoad.current && deps.some(Boolean)) {
      endRef.current?.scrollIntoView({ behavior: 'instant' });
      isFirstLoad.current = false;
    }
  }, deps);

  useEffect(() => {
    if (!isFirstLoad.current && shouldAutoScroll) {
      endRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  });

  return {
    containerRef,
    endRef,
    shouldAutoScroll,
    handleScroll,
    scrollToBottom,
  };
}
