import { X, FileText, Copy, Check, Pencil, Save, Eye, Loader2 } from "lucide-react";
import { useState, useCallback, useEffect, useRef } from "react";
import { useFileStore } from "../../stores/useFileStore";
import { useUIStore, getEffectiveTheme } from "../../stores/useUIStore";
import { CodeMirrorEditor } from "../ui/CodeMirrorEditor";

export function CodeViewer() {
  const openFilePath = useFileStore((s) => s.openFilePath);
  const openFileContent = useFileStore((s) => s.openFileContent);
  const openFileLanguage = useFileStore((s) => s.openFileLanguage);
  const isDirty = useFileStore((s) => s.isDirty);
  const isSaving = useFileStore((s) => s.isSaving);
  const closeFile = useFileStore((s) => s.closeFile);
  const updateFileContent = useFileStore((s) => s.updateFileContent);
  const saveFile = useFileStore((s) => s.saveFile);
  const [copied, setCopied] = useState(false);
  const [isEditing, setIsEditing] = useState(false);
  const theme = useUIStore((s) => s.theme);

  const isDark = getEffectiveTheme(theme) === "dark";

  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    // Reset editing state when file changes
    setIsEditing(false);
  }, [openFilePath]);

  // Listen for save events from CodeMirrorEditor
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const handleSave = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      if (detail?.content) {
        updateFileContent(detail.content);
        saveFile().catch(() => {});
      }
    };

    el.addEventListener("editor-save", handleSave);
    return () => el.removeEventListener("editor-save", handleSave);
  }, [updateFileContent, saveFile]);

  const handleCopy = useCallback(() => {
    if (openFileContent) {
      navigator.clipboard.writeText(openFileContent);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  }, [openFileContent]);

  const toggleEdit = useCallback(() => {
    if (isEditing) {
      // Switching from edit to view mode - confirm if dirty?
      // For now just switch, user can re-open to get original
    }
    setIsEditing((prev) => !prev);
  }, [isEditing]);

  const handleEditorChange = useCallback(
    (value: string) => {
      updateFileContent(value);
    },
    [updateFileContent],
  );

  const handleSaveClick = useCallback(() => {
    saveFile().catch((err) => {
      console.error("Save failed:", err);
    });
  }, [saveFile]);

  if (!openFilePath) {
    return (
      <div className="flex h-full items-center justify-center text-xs text-neutral-400">
        Select a file to view
      </div>
    );
  }

  return (
    <div ref={containerRef} className="flex h-full flex-col">
      {/* Tab header */}
      <div className="flex items-center justify-between border-b border-neutral-200 bg-neutral-50 px-3 py-1.5 dark:border-neutral-800 dark:bg-neutral-900">
        <div className="flex min-w-0 items-center gap-2">
          <FileText className="h-4 w-4 shrink-0 text-neutral-400" />
          <span className="truncate text-xs font-medium text-neutral-700 dark:text-neutral-300">
            {openFilePath}
          </span>
          {isDirty && (
            <span className="rounded bg-amber-100 px-1.5 py-0.5 text-[10px] font-medium text-amber-700 dark:bg-amber-900/40 dark:text-amber-400">
              Modified
            </span>
          )}
          {openFileLanguage && (
            <span className="shrink-0 rounded bg-neutral-200 px-1.5 py-0.5 text-[10px] font-medium uppercase text-neutral-600 dark:bg-neutral-700 dark:text-neutral-400">
              {openFileLanguage}
            </span>
          )}
        </div>
        <div className="flex items-center gap-1">
          {isSaving && (
            <span className="flex items-center gap-1 text-[10px] text-neutral-400">
              <Loader2 className="h-3 w-3 animate-spin" />
              Saving...
            </span>
          )}

          {/* Edit / View toggle */}
          <button
            onClick={toggleEdit}
            className={`rounded p-1 ${
              isEditing
                ? "bg-blue-100 text-blue-600 dark:bg-blue-900/40 dark:text-blue-400"
                : "text-neutral-400 hover:bg-neutral-200 hover:text-neutral-600 dark:hover:bg-neutral-700 dark:hover:text-neutral-300"
            }`}
            aria-label={isEditing ? "View mode" : "Edit mode"}
            title={isEditing ? "Switch to view mode" : "Switch to edit mode"}
          >
            {isEditing ? (
              <Eye className="h-3.5 w-3.5" />
            ) : (
              <Pencil className="h-3.5 w-3.5" />
            )}
          </button>

          {/* Save button (only in edit mode and when dirty) */}
          {isEditing && isDirty && (
            <button
              onClick={handleSaveClick}
              className="rounded p-1 text-green-600 hover:bg-green-100 dark:text-green-400 dark:hover:bg-green-900/30"
              aria-label="Save file"
              title="Save (Ctrl+S)"
            >
              <Save className="h-3.5 w-3.5" />
            </button>
          )}

          <button
            onClick={handleCopy}
            className="rounded p-1 text-neutral-400 hover:bg-neutral-200 hover:text-neutral-600 dark:hover:bg-neutral-700 dark:hover:text-neutral-300"
            aria-label="Copy file content"
          >
            {copied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
          </button>
          <button
            onClick={closeFile}
            className="rounded p-1 text-neutral-400 hover:bg-neutral-200 hover:text-neutral-600 dark:hover:bg-neutral-700 dark:hover:text-neutral-300"
            aria-label="Close file"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>

      {/* CodeMirror editor */}
      <div className="flex-1 overflow-hidden">
        {openFileContent !== null && (
          <CodeMirrorEditor
            value={openFileContent}
            onChange={handleEditorChange}
            filePath={openFilePath}
            readOnly={!isEditing}
            dark={isDark}
          />
        )}
      </div>
    </div>
  );
}
