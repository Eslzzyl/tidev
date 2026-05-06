import { useState, useRef, useEffect } from "react";

interface GoToLineDialogProps {
  totalLines: number;
  currentLine: number;
  onGo: (line: number) => void;
  onClose: () => void;
}

export function GoToLineDialog({
  totalLines,
  currentLine,
  onGo,
  onClose,
}: GoToLineDialogProps) {
  const [lineStr, setLineStr] = useState(String(currentLine));
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const line = parseInt(lineStr, 10);
    if (!isNaN(line) && line >= 1) {
      onGo(line);
      onClose();
    }
  };

  return (
    <div
      className="fixed inset-0 z-[9998] flex items-start justify-center pt-[15vh] bg-black/30"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="w-56 rounded-lg border border-neutral-200 bg-white p-3 shadow-xl dark:border-neutral-700 dark:bg-neutral-900">
        <form onSubmit={handleSubmit}>
          <div className="mb-2 text-xs font-medium text-neutral-700 dark:text-neutral-300">
            Go to line
          </div>
          <div className="flex items-center gap-1">
            <input
              ref={inputRef}
              type="text"
              value={lineStr}
              onChange={(e) => {
                const val = e.target.value.replace(/\D/g, "");
                setLineStr(val);
              }}
              className="w-full rounded border border-neutral-300 bg-white px-2 py-1 text-xs outline-none focus:border-blue-400 dark:border-neutral-600 dark:bg-neutral-800 dark:text-neutral-100 dark:focus:border-blue-500"
            />
            <span className="shrink-0 text-[11px] text-neutral-400">
              / {totalLines}
            </span>
          </div>
          <div className="mt-2 flex justify-end gap-1">
            <button
              type="button"
              onClick={onClose}
              className="rounded px-2 py-0.5 text-[11px] text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-800"
            >
              Cancel
            </button>
            <button
              type="submit"
              className="rounded bg-blue-600 px-2 py-0.5 text-[11px] text-white hover:bg-blue-700"
            >
              Go
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
