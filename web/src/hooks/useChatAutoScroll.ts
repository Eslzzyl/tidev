import { useEffect, useRef, useState, useCallback } from "react";

export interface UseChatAutoScrollResult {
  /** Attach to the scroll container's `onScroll`. */
  handleScroll: () => void;
  /** Scroll to the bottom of the message list. */
  scrollToBottom: (instant?: boolean) => void;
  /** True when user has scrolled up and is more than threshold from bottom. */
  showScrollButton: boolean;
  /** Reference to the end sentinel element (used for auto-scroll target). */
  endRef: React.RefObject<HTMLDivElement | null>;
}

/**
 * Enhanced auto-scroll hook for the chat message list.
 *
 * - Detects when the user scrolls away from the bottom (→ showScrollButton).
 * - Scrolls to bottom when new content arrives and user is pinned to bottom.
 * - Works correctly with virtualized lists.
 */
export function useChatAutoScroll(
  containerRef: React.RefObject<HTMLDivElement | null>,
  isStreaming: boolean,
): UseChatAutoScrollResult {
  const [showScrollButton, setShowScrollButton] = useState(false);
  const isPinnedRef = useRef(true);
  const endRef = useRef<HTMLDivElement | null>(null);

  // Threshold ratio: 10% of container height, capped to [24 ... 200] px.
  const getThreshold = useCallback(() => {
    const el = containerRef.current;
    if (!el || el.clientHeight <= 0) return 0;
    const raw = el.clientHeight * 0.1;
    return Math.max(24, Math.min(200, raw));
  }, [containerRef]);

  const handleScroll = useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    const { scrollHeight, scrollTop, clientHeight } = el;
    const threshold = getThreshold();
    const nearBottom = scrollHeight - scrollTop - clientHeight < threshold;
    isPinnedRef.current = nearBottom;
    setShowScrollButton(!nearBottom);
  }, [containerRef, getThreshold]);

  const scrollToBottom = useCallback(
    (instant = false) => {
      const el = containerRef.current;
      if (!el) return;
      el.scrollTo({
        top: el.scrollHeight,
        behavior: instant ? "instant" : "smooth",
      });
      isPinnedRef.current = true;
      setShowScrollButton(false);
    },
    [containerRef],
  );

  // Auto-scroll loop during streaming.
  // Uses a continuous requestAnimationFrame loop so it fires on EVERY frame
  // while streaming is active, not just when React dependencies happen to change.
  // This ensures the viewport stays pinned to the bottom as content grows
  // incrementally within a single round (text chunks, tool calls, etc.).
  useEffect(() => {
    if (!isStreaming) return;

    let rafId: number;
    const tick = () => {
      if (isPinnedRef.current && containerRef.current) {
        containerRef.current.scrollTo({
          top: containerRef.current.scrollHeight,
          behavior: "instant",
        });
      }
      rafId = requestAnimationFrame(tick);
    };
    rafId = requestAnimationFrame(tick);

    return () => cancelAnimationFrame(rafId);
  }, [isStreaming, containerRef]);

  return {
    handleScroll,
    scrollToBottom,
    showScrollButton,
    endRef,
  };
}
