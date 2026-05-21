import { useState, useEffect } from "react";
import type { VirtualItem } from "@tanstack/react-virtual";
import type { Round, SystemMessageBlock } from "../../types/round";
import { MessageRound } from "./MessageRound";
import { SystemMessageBlock as SystemMessageBlockComponent } from "../renderers/SystemMessageBlock";

export type VirtualEntry = Round | SystemMessageBlock;

interface Props {
  entries: VirtualEntry[];
  virtualItems: VirtualItem[];
  totalSize: number;
  isVirtualized: boolean;
  measureElement: (el: Element | null) => void;
  onUndoRequest?: (messageId: string) => void;
  canUndo?: boolean;
}

/**
 * Renders the message list.
 *
 * For large lists (> VIRTUALIZE_THRESHOLD) it uses absolute-positioned
 * virtual rows via `@tanstack/react-virtual`.  For small lists it falls
 * back to a simple flat layout.
 *
 * Stagger entrance animations are only applied during the initial mount
 * of this component.  Tab switches and sidebar toggles do NOT re-trigger
 * the cascade, avoiding costly animation of every message.
 */
export function VirtualMessageList({
  entries,
  virtualItems,
  totalSize,
  isVirtualized,
  measureElement,
  onUndoRequest,
  canUndo,
}: Props) {
  // Track initial mount: stagger animations only fire once when the component
  // first appears (e.g. loading a session for the first time).  Subsequent
  // re-mounts from tab switches or sidebar toggles skip the cascade entirely.
  const [initialMount, setInitialMount] = useState(true);
  useEffect(() => {
    if (initialMount) {
      const timer = setTimeout(() => setInitialMount(false), 600);
      return () => clearTimeout(timer);
    }
  }, [initialMount]);

  const getStaggerIndex = (idx: number) => (initialMount ? idx : undefined);

  // --- Small list: flat render (no virtualization) ---
  if (!isVirtualized) {
    return (
      <div className="divide-y divide-neutral-100 dark:divide-neutral-900">
        {entries.map((entry, idx) => (
          <div
            key={entry.id}
            className="contents"
            style={{ contentVisibility: "auto" }}
          >
            {renderEntry(entry, onUndoRequest, canUndo, getStaggerIndex(idx))}
          </div>
        ))}
      </div>
    );
  }

  // --- Virtualized list ---
  return (
    <div
      style={{ position: "relative", height: `${totalSize}px`, width: "100%" }}
    >
      <div className="divide-y divide-neutral-100 dark:divide-neutral-900">
        {virtualItems.map((virtualItem) => {
          const entry = entries[virtualItem.index];
          if (!entry) return null;

          return (
            <div
              key={virtualItem.key}
              data-index={virtualItem.index}
              ref={measureElement}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${virtualItem.start}px)`,
                contain: "layout style paint",
              }}
            >
              {renderEntry(
                entry,
                onUndoRequest,
                canUndo,
                getStaggerIndex(virtualItem.index),
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function renderEntry(
  entry: VirtualEntry,
  onUndoRequest?: (messageId: string) => void,
  canUndo?: boolean,
  staggerIndex?: number,
) {
  if ("kind" in entry && entry.kind === "system") {
    return <SystemMessageBlockComponent message={entry.message} />;
  }
  return (
    <MessageRound
      round={entry as Round}
      onUndoRequest={onUndoRequest}
      canUndo={canUndo}
      staggerIndex={staggerIndex}
    />
  );
}
