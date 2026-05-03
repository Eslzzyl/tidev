import { useState, useEffect, useRef } from "react";
import { X, Check } from "lucide-react";

interface RenameDialogProps {
  isOpen: boolean;
  currentTitle: string;
  onClose: () => void;
  onConfirm: (title: string) => void;
  isLoading?: boolean;
}

export function RenameDialog({
  isOpen,
  currentTitle,
  onClose,
  onConfirm,
  isLoading = false,
}: RenameDialogProps) {
  const [title, setTitle] = useState(currentTitle);
  const inputRef = useRef<HTMLInputElement>(null);

  // Focus and select input when dialog opens
  useEffect(() => {
    if (isOpen) {
      const raf = requestAnimationFrame(() => {
        inputRef.current?.focus();
        inputRef.current?.select();
      });
      return () => cancelAnimationFrame(raf);
    }
  }, [isOpen]);

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleConfirm();
    }
    if (e.key === "Escape") {
      onClose();
    }
  }

  function handleConfirm() {
    const trimmed = title.trim();
    if (trimmed) {
      onConfirm(trimmed);
    }
  }

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      <div className="w-full max-w-md rounded-lg bg-white shadow-lg dark:bg-neutral-900">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-neutral-200 px-4 py-3 dark:border-neutral-800">
          <h3 className="text-lg font-semibold text-neutral-900 dark:text-neutral-100">
            Rename Session
          </h3>
          <button
            onClick={onClose}
            disabled={isLoading}
            className="rounded p-1 text-neutral-400 hover:bg-neutral-100 hover:text-neutral-600 disabled:opacity-50 dark:hover:bg-neutral-800 dark:hover:text-neutral-300"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Content */}
        <div className="px-4 py-4">
          <label className="mb-1.5 block text-xs font-medium text-neutral-500 dark:text-neutral-400">
            Session title
          </label>
          <input
            ref={inputRef}
            type="text"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Enter new session title..."
            disabled={isLoading}
            className="w-full rounded-lg border border-neutral-300 bg-white px-3 py-2 text-sm text-neutral-900 placeholder-neutral-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 disabled:opacity-50 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100 dark:placeholder-neutral-500 dark:focus:border-blue-400"
          />
        </div>

        {/* Footer */}
        <div className="flex justify-end gap-2 border-t border-neutral-200 px-4 py-3 dark:border-neutral-800">
          <button
            onClick={onClose}
            disabled={isLoading}
            className="rounded px-4 py-2 text-sm font-medium text-neutral-600 hover:bg-neutral-100 disabled:opacity-50 dark:text-neutral-400 dark:hover:bg-neutral-800"
          >
            Cancel
          </button>
          <button
            onClick={handleConfirm}
            disabled={isLoading || !title.trim()}
            className="flex items-center gap-1.5 rounded bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50 dark:bg-blue-700 dark:hover:bg-blue-600"
          >
            {isLoading ? (
              <span className="flex items-center gap-2">
                <span className="h-4 w-4 animate-spin rounded-full border-2 border-white border-t-transparent" />
                Renaming...
              </span>
            ) : (
              <>
                <Check className="h-4 w-4" />
                Rename
              </>
            )}
          </button>
        </div>
      </div>
    </div>
  );
}
