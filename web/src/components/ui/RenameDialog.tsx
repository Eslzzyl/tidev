import { useState, useRef, useEffect } from "react";
import { useTranslation } from "react-i18next";

interface RenameDialogProps {
  currentName: string;
  onSubmit: (newName: string) => void;
  onClose: () => void;
}

export function RenameDialog({ currentName, onSubmit, onClose }: RenameDialogProps) {
  const { t } = useTranslation();
  const [name, setName] = useState(currentName);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    // Select the name without extension for files
    const dotIndex = currentName.lastIndexOf(".");
    if (dotIndex > 0) {
      inputRef.current?.setSelectionRange(0, dotIndex);
    } else {
      inputRef.current?.select();
    }
  }, [currentName]);

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
    if (trimmed && trimmed !== currentName) {
      onSubmit(trimmed);
    } else {
      onClose();
    }
  };

  return (
    <div
      className="fixed inset-0 z-[9998] flex items-center justify-center bg-black/30"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="w-72 rounded-lg border border-neutral-200 bg-white p-4 shadow-xl dark:border-neutral-700 dark:bg-neutral-900">
        <div className="mb-3 text-sm font-medium text-neutral-800 dark:text-neutral-200">
          {t("Rename")}
        </div>

        <form onSubmit={handleSubmit}>
          <input
            ref={inputRef}
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="w-full rounded border border-neutral-300 bg-white px-2 py-1.5 text-base outline-none focus:border-blue-400 dark:border-neutral-600 dark:bg-neutral-800 dark:text-neutral-100 dark:focus:border-blue-500"
          />
          <div className="mt-3 flex justify-end gap-2">
            <button
              type="button"
              onClick={onClose}
              className="rounded px-3 py-1 text-xs text-neutral-600 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
            >
              {t("Cancel")}
            </button>
            <button
              type="submit"
              disabled={!name.trim() || name.trim() === currentName}
              className="rounded bg-blue-600 px-3 py-1 text-xs text-white hover:bg-blue-700 disabled:opacity-40"
            >
              Rename
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
