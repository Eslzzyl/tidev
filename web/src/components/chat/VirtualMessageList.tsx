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
  // --- Small list: flat render (no virtualization) ---
  if (!isVirtualized) {
    return (
      <div className="divide-y divide-neutral-100 dark:divide-neutral-900">
        {entries.map((entry) => (
          <div key={entry.id} className="contents">
            {renderEntry(entry, onUndoRequest, canUndo)}
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
              }}
            >
              {renderEntry(entry, onUndoRequest, canUndo)}
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
) {
  if ("kind" in entry && entry.kind === "system") {
    return (
      <SystemMessageBlockComponent message={entry.message} />
    );
  }
  return (
    <MessageRound
      round={entry as Round}
      onUndoRequest={onUndoRequest}
      canUndo={canUndo}
    />
  );
}
