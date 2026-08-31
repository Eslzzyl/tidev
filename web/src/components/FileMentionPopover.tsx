import { useEffect, useState } from "react";
import {
  Database,
  File as FileIcon,
  FileArchive,
  FileBraces,
  FileCode,
  FileImage,
  FileMusic,
  FileSpreadsheet,
  FileText,
  FileVideoCamera,
  Folder,
  type LucideIcon,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { api } from "../api/client";
import type { FileSuggestion } from "../types/api";
import { Button } from "./ui";

interface Props {
  query: string;
  onSelect: (path: string) => void;
  onClose: () => void;
  selectedIndex: number;
  onSelectedIndexChange: (index: number) => void;
}

export function FileMentionPopover({
  query,
  onSelect,
  onClose,
  selectedIndex,
  onSelectedIndexChange,
}: Props) {
  const { t } = useTranslation();
  const [suggestions, setSuggestions] = useState<FileSuggestion[]>([]);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    const timer = setTimeout(async () => {
      setLoading(true);
      try {
        const result = await api.searchFiles(query);
        setSuggestions(result.files.slice(0, 20));
        onSelectedIndexChange(0);
      } catch {
        setSuggestions([]);
      } finally {
        setLoading(false);
      }
    }, 150);
    return () => clearTimeout(timer);
  }, [query, onSelectedIndexChange]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (suggestions.length === 0) return;
      if (event.key === "ArrowDown") {
        event.preventDefault();
        onSelectedIndexChange((selectedIndex + 1) % suggestions.length);
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        onSelectedIndexChange((selectedIndex - 1 + suggestions.length) % suggestions.length);
      } else if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        const chosen = suggestions[selectedIndex];
        if (chosen) onSelect(chosen.path);
      } else if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [suggestions, selectedIndex, onSelect, onClose, onSelectedIndexChange]);

  if (loading && suggestions.length === 0) {
    return (
      <div className="composer-popover file-mention-popover">
        <div className="file-mention-loading">{t("Searching…")}</div>
      </div>
    );
  }

  if (suggestions.length === 0) {
    return (
      <div className="composer-popover file-mention-popover">
        <div className="file-mention-empty">{t("No files found")}</div>
        <Button
          type="button"
          className="composer-option"
          onClick={onClose}
          variant="ghost"
          size="sm"
        >
          {t("Close")}
        </Button>
      </div>
    );
  }

  return (
    <div className="composer-popover file-mention-popover">
      {suggestions.map((item, idx) => (
        <Button
          key={item.path}
          className={idx === selectedIndex ? "composer-option selected" : "composer-option"}
          onMouseEnter={() => onSelectedIndexChange(idx)}
          onClick={() => onSelect(item.path)}
          variant="ghost"
          size="sm"
        >
          <span className="file-mention-path">
            {highlightMatches(item.display || item.path, item.matched_indices)}
          </span>
          <FileMentionIcon suggestion={item} />
        </Button>
      ))}
    </div>
  );
}

interface FileIconInfo {
  icon: LucideIcon;
  label: string;
  tone: string;
}

const CODE_EXTENSIONS = new Set([
  "c",
  "cc",
  "cpp",
  "cs",
  "css",
  "go",
  "h",
  "hpp",
  "html",
  "java",
  "js",
  "jsx",
  "kt",
  "less",
  "mjs",
  "php",
  "py",
  "rb",
  "rs",
  "scss",
  "sh",
  "sql",
  "swift",
  "ts",
  "tsx",
  "vue",
  "xml",
  "zig",
]);

const CONFIG_EXTENSIONS = new Set(["env", "ini", "properties", "toml", "yaml", "yml"]);
const IMAGE_EXTENSIONS = new Set([
  "avif",
  "bmp",
  "gif",
  "ico",
  "jpeg",
  "jpg",
  "png",
  "svg",
  "webp",
]);
const ARCHIVE_EXTENSIONS = new Set(["7z", "bz2", "gz", "rar", "tar", "xz", "zip"]);
const AUDIO_EXTENSIONS = new Set(["flac", "m4a", "mp3", "ogg", "wav"]);
const VIDEO_EXTENSIONS = new Set(["avi", "mkv", "mov", "mp4", "webm"]);
const SPREADSHEET_EXTENSIONS = new Set(["csv", "ods", "xls", "xlsx"]);
const DATA_EXTENSIONS = new Set(["db", "graphql", "sqlite"]);
const TEXT_EXTENSIONS = new Set(["log", "md", "mdx", "rst", "tex", "txt"]);

function FileMentionIcon({ suggestion }: { suggestion: FileSuggestion }) {
  const { icon: Icon, label, tone } = getFileIcon(suggestion);

  return (
    <span className={`file-mention-kind ${tone}`} role="img" aria-label={label} title={label}>
      <Icon size={14} strokeWidth={1.8} aria-hidden="true" />
    </span>
  );
}

function getFileIcon(suggestion: FileSuggestion): FileIconInfo {
  if (suggestion.kind === "directory") {
    return { icon: Folder, label: "Directory", tone: "directory" };
  }
  if (suggestion.kind === "image") {
    return { icon: FileImage, label: "Image", tone: "image" };
  }

  const fileName = suggestion.path.split(/[\\/]/).pop()?.toLowerCase() ?? "";
  const extension = fileName.includes(".") ? (fileName.split(".").pop() ?? "") : "";

  if (fileName === "dockerfile" || fileName.startsWith("dockerfile.")) {
    return { icon: FileCode, label: "Code file", tone: "code" };
  }
  if (CODE_EXTENSIONS.has(extension)) {
    return { icon: FileCode, label: "Code file", tone: "code" };
  }
  if (extension === "json" || extension === "jsonc") {
    return { icon: FileBraces, label: "JSON file", tone: "config" };
  }
  if (CONFIG_EXTENSIONS.has(extension)) {
    return { icon: FileBraces, label: "Configuration file", tone: "config" };
  }
  if (DATA_EXTENSIONS.has(extension)) {
    return { icon: Database, label: "Data file", tone: "data" };
  }
  if (IMAGE_EXTENSIONS.has(extension)) {
    return { icon: FileImage, label: "Image", tone: "image" };
  }
  if (ARCHIVE_EXTENSIONS.has(extension)) {
    return { icon: FileArchive, label: "Archive", tone: "archive" };
  }
  if (AUDIO_EXTENSIONS.has(extension)) {
    return { icon: FileMusic, label: "Audio file", tone: "audio" };
  }
  if (VIDEO_EXTENSIONS.has(extension)) {
    return { icon: FileVideoCamera, label: "Video file", tone: "video" };
  }
  if (SPREADSHEET_EXTENSIONS.has(extension)) {
    return { icon: FileSpreadsheet, label: "Spreadsheet", tone: "spreadsheet" };
  }
  if (TEXT_EXTENSIONS.has(extension)) {
    return { icon: FileText, label: "Text file", tone: "text" };
  }

  return { icon: FileIcon, label: "File", tone: "file" };
}

function highlightMatches(display: string, indices: number[]): React.ReactNode {
  if (!indices || indices.length === 0) return display;
  const sorted = [...indices].sort((a, b) => a - b);
  const result: React.ReactNode[] = [];
  let last = 0;
  for (const i of sorted) {
    if (i < 0 || i >= display.length) continue;
    if (i > last) result.push(<span key={`t-${i}`}>{display.slice(last, i)}</span>);
    result.push(
      <strong key={`m-${i}`} className="file-mention-match">
        {display[i]}
      </strong>,
    );
    last = i + 1;
  }
  if (last < display.length) result.push(<span key="t-end">{display.slice(last)}</span>);
  return <>{result}</>;
}
