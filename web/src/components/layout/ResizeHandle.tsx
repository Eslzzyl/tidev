import { useCallback } from "react";

interface Props {
  onResizeStart: (e: React.MouseEvent) => void;
  isResizing?: boolean;
}

export function ResizeHandle({ onResizeStart, isResizing }: Props) {
  return (
    <div
      onMouseDown={onResizeStart}
      className={`
        group relative hidden w-2 cursor-col-resize md:block
        before:absolute before:inset-y-0 before:left-1/2 before:w-px before:-translate-x-1/2
        before:bg-transparent before:transition-colors
        hover:before:bg-neutral-300 dark:hover:before:bg-neutral-600
        ${isResizing ? "before:bg-neutral-400 dark:before:bg-neutral-500" : ""}
      `}
      role="separator"
      aria-label="Resize sidebar"
    >
      {/* Visual grabber dot — only shows on hover */}
      <div className="absolute inset-y-1/2 left-1/2 h-6 w-0.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-neutral-300/0 transition-all group-hover:bg-neutral-300/40 dark:bg-neutral-600/0 dark:group-hover:bg-neutral-600/40" />
    </div>
  );
}
