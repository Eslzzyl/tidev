import { useCallback } from "react";

interface Props {
  onResizeStart: (e: React.MouseEvent) => void;
  isResizing?: boolean;
}

export function ResizeHandle({ onResizeStart, isResizing }: Props) {
  return (
    <div
      onMouseDown={onResizeStart}
      className={`hidden w-1 cursor-col-resize bg-transparent hover:bg-neutral-300 dark:hover:bg-neutral-700 md:block ${
        isResizing ? "bg-neutral-400 dark:bg-neutral-600" : ""
      }`}
      role="separator"
      aria-label="Resize sidebar"
    />
  );
}
