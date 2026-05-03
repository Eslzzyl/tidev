import { useState, useEffect, useRef, useCallback } from 'react';
import {
  Folder, Image, File, FileCode, FileText, FileJson, FileCog,
  FileTerminal, FileType, FileImage, FileLock2, FileArchive,
  FileVideo, FileAudio, FileSpreadsheet, FileWarning,
} from 'lucide-react';
import { api } from '../../api/client';
import type { FileSuggestion } from '../../types/api';

interface FileMentionPopoverProps {
  query: string;
  onSelect: (path: string, kind: FileSuggestion['kind']) => void;
  onClose: () => void;
  position: { x: number; y: number };
}

interface FileSuggestionWithIcon extends FileSuggestion {
  icon: React.ReactNode;
}

export function FileMentionPopover({ query, onSelect, onClose, position }: FileMentionPopoverProps) {
  const [suggestions, setSuggestions] = useState<FileSuggestionWithIcon[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [loading, setLoading] = useState(false);
  const popoverRef = useRef<HTMLDivElement>(null);

  // Debounced search
  useEffect(() => {
    const timeoutId = setTimeout(async () => {
      setLoading(true);
      try {
        const response = await api.searchFiles(query);
        const suggestionsWithIcons = response.suggestions.map((s) => ({
          ...s,
          icon: getFileIcon(s.kind, s.path),
        }));
        setSuggestions(suggestionsWithIcons);
        setSelectedIndex(0);
      } catch (error) {
        console.error('Failed to search files:', error);
        setSuggestions([]);
      } finally {
        setLoading(false);
      }
    }, 150);

    return () => clearTimeout(timeoutId);
  }, [query]);

  // Handle keyboard navigation
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (suggestions.length === 0) return;

      switch (e.key) {
        case 'ArrowDown':
          e.preventDefault();
          setSelectedIndex((prev) => (prev + 1) % suggestions.length);
          break;
        case 'ArrowUp':
          e.preventDefault();
          setSelectedIndex((prev) => (prev - 1 + suggestions.length) % suggestions.length);
          break;
        case 'Enter':
        case 'Tab':
          e.preventDefault();
          if (suggestions[selectedIndex]) {
            const suggestion = suggestions[selectedIndex];
            onSelect(suggestion.path, suggestion.kind);
          }
          break;
        case 'Escape':
          e.preventDefault();
          onClose();
          break;
      }
    };

    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [suggestions, selectedIndex, onSelect, onClose]);

  // Handle click outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (popoverRef.current && !popoverRef.current.contains(e.target as Node)) {
        onClose();
      }
    };

    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [onClose]);

  // Scroll selected item into view
  const selectedRef = useCallback((node: HTMLDivElement | null) => {
    if (node) {
      node.scrollIntoView({ block: 'nearest' });
    }
  }, []);

  // Position popover above the @ character with bottom edge touching the @ line
  const bottom = window.innerHeight - position.y;
  const availableAbove = position.y - 8; // leave 8px margin from viewport top
  const maxHeight = Math.min(256, Math.max(100, availableAbove));

  return (
    <div
      ref={popoverRef}
      className="fixed z-50 w-80 overflow-hidden rounded-lg border border-neutral-200 bg-white shadow-lg dark:border-neutral-700 dark:bg-neutral-800"
      style={{
        left: Math.min(position.x, window.innerWidth - 320),
        bottom: Math.max(8, bottom),
        maxHeight: `${maxHeight}px`,
      }}
    >
      {/* Header */}
      <div className="flex items-center justify-between border-b border-neutral-100 px-3 py-2 dark:border-neutral-700">
        <span className="text-xs font-medium text-neutral-500 dark:text-neutral-400">
          Files {query ? `· @${query}` : ''}
        </span>
        {loading && (
          <span className="text-xs text-neutral-400 dark:text-neutral-500">Loading...</span>
        )}
      </div>

      {/* Suggestions list */}
      <div className="max-h-52 overflow-y-auto py-1">
        {suggestions.length === 0 && !loading && (
          <div className="px-3 py-2 text-sm text-neutral-500 dark:text-neutral-400">
            No files found
          </div>
        )}
        {suggestions.map((suggestion, index) => (
          <div
            key={suggestion.path}
            ref={index === selectedIndex ? selectedRef : null}
            onClick={() => onSelect(suggestion.path, suggestion.kind)}
            className={`flex cursor-pointer items-center gap-2 px-3 py-2 text-sm ${
              index === selectedIndex
                ? 'bg-neutral-100 dark:bg-neutral-700'
                : 'hover:bg-neutral-50 dark:hover:bg-neutral-750'
            }`}
          >
            <span className="flex-shrink-0 text-neutral-500 dark:text-neutral-400">
              {suggestion.icon}
            </span>
            <span className="flex-1 truncate font-mono text-neutral-700 dark:text-neutral-300">
              {highlightMatches(suggestion.display, suggestion.matched_indices)}
            </span>
          </div>
        ))}
      </div>

      {/* Footer hint */}
      <div className="border-t border-neutral-100 bg-neutral-50 px-3 py-1.5 text-xs text-neutral-400 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-500">
        <span className="mr-3">↑↓ to navigate</span>
        <span className="mr-3">↵ / Tab to select</span>
        <span>esc to close</span>
      </div>
    </div>
  );
}

function getFileIcon(kind: FileSuggestion['kind'], path: string): React.ReactNode {
  // Directory
  if (kind === 'directory') return <Folder className="h-4 w-4" />;
  // Image (classified by backend)
  if (kind === 'image') return <Image className="h-4 w-4" />;

  // Parse extension from path
  const ext = path.split('.').pop()?.toLowerCase() || '';

  switch (ext) {
    // Code files
    case 'rs':
    case 'ts':
    case 'tsx':
    case 'js':
    case 'jsx':
    case 'mjs':
    case 'cjs':
    case 'py':
    case 'go':
    case 'rb':
    case 'java':
    case 'kt':
    case 'swift':
    case 'c':
    case 'h':
    case 'cpp':
    case 'hpp':
    case 'cs':
    case 'php':
    case 'r':
    case 'scala':
    case 'zig':
      return <FileCode className="h-4 w-4" />;

    // Shell scripts
    case 'sh':
    case 'bash':
    case 'zsh':
    case 'fish':
      return <FileTerminal className="h-4 w-4" />;

    // JSON
    case 'json':
    case 'jsonc':
      return <FileJson className="h-4 w-4" />;

    // Config / data files
    case 'yaml':
    case 'yml':
    case 'toml':
    case 'env':
    case 'ini':
    case 'cfg':
    case 'conf':
      return <FileCog className="h-4 w-4" />;

    // Markdown / text
    case 'md':
    case 'mdx':
    case 'txt':
    case 'log':
      return <FileText className="h-4 w-4" />;

    // Stylesheets
    case 'css':
    case 'scss':
    case 'sass':
    case 'less':
      return <FileType className="h-4 w-4" />;

    // Image files (not caught by backend kind)
    case 'svg':
    case 'ico':
      return <FileImage className="h-4 w-4" />;

    // Lock files
    case 'lock':
      return <FileLock2 className="h-4 w-4" />;

    // Archives
    case 'zip':
    case 'tar':
    case 'gz':
    case 'tgz':
    case 'bz2':
    case 'xz':
    case 'rar':
    case '7z':
      return <FileArchive className="h-4 w-4" />;

    // Video
    case 'mp4':
    case 'avi':
    case 'mov':
    case 'mkv':
    case 'webm':
      return <FileVideo className="h-4 w-4" />;

    // Audio
    case 'mp3':
    case 'wav':
    case 'flac':
    case 'ogg':
    case 'aac':
      return <FileAudio className="h-4 w-4" />;

    // Spreadsheets
    case 'csv':
    case 'xlsx':
    case 'xls':
      return <FileSpreadsheet className="h-4 w-4" />;

    // Binary / compiled
    case 'wasm':
    case 'so':
    case 'dylib':
    case 'dll':
    case 'exe':
    case 'bin':
      return <FileWarning className="h-4 w-4" />;

    // Default
    default:
      return <File className="h-4 w-4" />;
  }
}

function highlightMatches(display: string, matchedIndices: number[]): React.ReactNode {
  if (!matchedIndices || matchedIndices.length === 0) {
    return display;
  }

  const result: React.ReactNode[] = [];
  let lastIndex = 0;

  for (const index of matchedIndices) {
    if (index > lastIndex) {
      result.push(
        <span key={`text-${index}`}>{display.slice(lastIndex, index)}</span>
      );
    }
    result.push(
      <span key={`match-${index}`} className="font-bold text-neutral-900 dark:text-neutral-100">
        {display[index]}
      </span>
    );
    lastIndex = index + 1;
  }

  if (lastIndex < display.length) {
    result.push(<span key={`text-end`}>{display.slice(lastIndex)}</span>);
  }

  return result;
}
