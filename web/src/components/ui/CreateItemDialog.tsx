import { useState, useRef, useEffect } from "react";
import { X, File, Folder } from "lucide-react";

interface CreateItemDialogProps {
  /** Parent directory path where the item will be created */
  parentPath: string;
  /** Type of item to create */
  type: "file" | "directory";
  /** Called with the new item name */
  onSubmit: (name: string) => void;
  /** Called to close the dialog */
  onClose: () => void;
}

export function CreateItemDialog({ parentPath, type, onSubmit, onClose }: CreateItemDialogProps) {
  const [name, setName] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
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
    const trimmed = name.trim();
    if (trimmed) {
      onSubmit(trimmed);
    }
  };

  return (
    <div
      className="fixed inset-0 z-[9998] flex items-center justify-center bg-black/30"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="w-80 rounded-lg border border-neutral-200 bg-white p-4 shadow-xl dark:border-neutral-700 dark:bg-neutral-900">
        <div className="mb-3 flex items-center justify-between">
          <div className="flex items-center gap-2">
            {type === "file" ? (
              <File className="h-4 w-4 text-blue-500" />
            ) : (
              <Folder className="h-4 w-4 text-yellow-500" />
            )}
            <span className="text-sm font-medium text-neutral-800 dark:text-neutral-200">
              New {type}
            </span>
          </div>
          <button
            onClick={onClose}
            className="rounded p-1 text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>

        <div className="mb-2 text-[11px] text-neutral-500">
          in <span className="font-mono">{parentPath || "/"}</span>
        </div>

        <form onSubmit={handleSubmit}>
          <input
            ref={inputRef}
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={type === "file" ? "filename.ext" : "directory-name"}
            className="w-full rounded border border-neutral-300 bg-white px-2 py-1.5 text-base outline-none focus:border-blue-400 dark:border-neutral-600 dark:bg-neutral-800 dark:text-neutral-100 dark:focus:border-blue-500"
          />
          <div className="mt-3 flex justify-end gap-2">
            <button
              type="button"
              onClick={onClose}
              className="rounded px-3 py-1 text-xs text-neutral-600 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={!name.trim()}
              className="rounded bg-blue-600 px-3 py-1 text-xs text-white hover:bg-blue-700 disabled:opacity-40"
            >
              Create
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
