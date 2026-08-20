import { useCallback, useRef } from "react";
import {
  useVirtualizer,
  measureElement,
  type VirtualItem,
  type Virtualizer,
} from "@tanstack/react-virtual";
import type { Round, SystemMessageBlock, ShellBlock } from "../types/round";

/** Only enable virtual scrolling when there are more than this many entries. */
const VIRTUALIZE_THRESHOLD = 30;

/** Number of extra items rendered above/below the visible viewport. */
const OVERSCAN = 6;

export type VirtualEntry = Round | SystemMessageBlock | ShellBlock;

export interface UseMessageVirtualizerResult {
  /** The @tanstack/react-virtual Virtualizer instance. */
  virtualizer: Virtualizer<HTMLDivElement, Element>;
  /** Currently visible virtual items (subset of entries). */
  virtualItems: VirtualItem[];
  /** Total estimated height of all entries (px). */
  totalSize: number;
  /** Whether virtualization is active (false for small lists). */
  isVirtualized: boolean;
  /** Ref assigned to each virtual row for dynamic measurement. */
  measureElement: (el: Element | null) => void;
}

/**
 * Estimate the pixel height of a message entry before it is rendered.
 * The `measureElement` callback will correct this after first paint.
 */
function estimateEntryHeight(entry: VirtualEntry | undefined): number {
  if (!entry) return 160;

  // Shell blocks
  if ("kind" in entry && entry.kind === "shell") {
    const shell = entry as ShellBlock;
    // Header ≈ 40px + output text ≈ 0.5px per char + padding
    let height = 80;
    const outputLen = shell.output.content?.length || 0;
    height += Math.min(outputLen * 0.5 + 20, 800);
    return height;
  }

  // System message blocks are compact
  if ("kind" in entry && entry.kind === "system") {
    return 80;
  }

  const round = entry as Round;
  // Base: header (avatar row + padding) ≈ 80px + 40px margin + 40px footer
  let height = 180;

  for (const seg of round.segments) {
    if (seg.type === "text") {
      // Rough heuristic: each character ≈ 0.5px, capped at 800px per segment
      height += Math.min(seg.content.length * 0.5 + 40, 800);
    } else if (seg.type === "reasoning") {
      // Collapsible thinking block (header + preview)
      height += 60;
    } else if (seg.type === "tool_call") {
      // Compact tool-call card with status icon
      height += 80;
    }
  }

  return Math.max(180, height);
}

/**
 * Hook that creates a `@tanstack/react-virtual` virtualizer for the
 * message / round list.
 *
 * Automatically disables virtual scrolling for small lists (< 50 entries).
 */
export function useMessageVirtualizer(
  containerRef: React.RefObject<HTMLDivElement | null>,
  entries: VirtualEntry[],
): UseMessageVirtualizerResult {
  // Stable reference to entries for the estimateSize callback
  const entriesRef = useRef(entries);
  entriesRef.current = entries;

  const isVirtualized = entries.length > VIRTUALIZE_THRESHOLD;

  const estimateSize = useCallback((index: number) => {
    return estimateEntryHeight(entriesRef.current[index]);
  }, []);

  const getItemKey = useCallback((index: number) => {
    return entriesRef.current[index]?.id ?? String(index);
  }, []);

  // eslint-disable-next-line react-hooks/incompatible-library -- useVirtualizer is stable, it manages its own memoization
  const virtualizer = useVirtualizer({
    count: entries.length,
    getScrollElement: () => containerRef.current,
    estimateSize,
    getItemKey,
    measureElement,
    useAnimationFrameWithResizeObserver: true,
    overscan: OVERSCAN,
    enabled: isVirtualized,
  });

  const virtualItems = virtualizer.getVirtualItems();

  const totalSize = virtualizer.getTotalSize();

  return {
    virtualizer,
    virtualItems,
    totalSize,
    isVirtualized,
    measureElement: virtualizer.measureElement,
  };
}
