import { X, FileIcon } from "lucide-react";
import type { OpenFile } from "../../stores/useFileStore";

interface FileTabsProps {
  files: OpenFile[];
  activePath: string | null;
  onSelect: (path: string) => void;
  onClose: (path: string, e: React.MouseEvent) => void;
}

export function FileTabs({ files, activePath, onSelect, onClose }: FileTabsProps) {
  if (files.length === 0) return null;

  return (
    <div className="flex items-center overflow-x-auto border-b border-neutral-200 bg-neutral-50 dark:border-neutral-800 dark:bg-neutral-900">
      {files.map((file) => {
        const isActive = file.path === activePath;
        return (
          <button
            key={file.path}
            onClick={() => onSelect(file.path)}
            className={`group flex shrink-0 items-center gap-1.5 border-r px-3 py-1.5 text-left text-xs transition-colors last:border-r-0 ${
              isActive
                ? "border-b-2 border-b-blue-500 bg-white text-blue-700 dark:bg-neutral-950 dark:text-blue-400"
                : "border-b border-transparent text-neutral-500 hover:bg-neutral-100 hover:text-neutral-700 dark:hover:bg-neutral-800 dark:hover:text-neutral-300"
            }`}
            title={file.path}
          >
            <FileIcon className="h-3 w-3 shrink-0 text-neutral-400" />
            <span className="max-w-[120px] truncate">
              {file.path.split("/").pop() || file.path}
            </span>
            {file.isDirty && <span className="h-2 w-2 shrink-0 rounded-full bg-amber-400" />}
            <span
              onClick={(e) => onClose(file.path, e)}
              className="ml-0.5 cursor-pointer rounded p-0.5 text-neutral-400 opacity-0 hover:bg-neutral-200 hover:text-neutral-600 group-hover:opacity-100 dark:hover:bg-neutral-700 dark:hover:text-neutral-300"
              role="button"
              aria-label={`Close ${file.path}`}
            >
              {" "}
              <X className="h-3 w-3" />
            </span>
          </button>
        );
      })}
    </div>
  );
}
