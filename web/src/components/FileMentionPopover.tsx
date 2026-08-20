import { useEffect, useState } from "react";
import { api } from "../api";

interface FileSuggestion {
  path: string;
  display: string;
  kind: string;
  matched_indices: number[];
}

interface Props {
  query: string;
  onSelect: (path: string) => void;
  onClose: () => void;
  selectedIndex: number;
  onSelectedIndexChange: (index: number) => void;
}

export function FileMentionPopover({ query, onSelect, onClose, selectedIndex, onSelectedIndexChange }: Props) {
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
        <div className="file-mention-loading">Searching…</div>
      </div>
    );
  }

  if (suggestions.length === 0) {
    return (
      <div className="composer-popover file-mention-popover">
        <div className="file-mention-empty">No files found</div>
        <button className="composer-option" onClick={onClose}>
          Close
        </button>
      </div>
    );
  }

  return (
    <div className="composer-popover file-mention-popover">
      {suggestions.map((item, idx) => (
        <button
          key={item.path}
          className={idx === selectedIndex ? "composer-option selected" : "composer-option"}
          onMouseEnter={() => onSelectedIndexChange(idx)}
          onClick={() => onSelect(item.path)}
        >
          <span className="file-mention-path">{highlightMatches(item.display || item.path, item.matched_indices)}</span>
          <small className="file-mention-kind">{item.kind}</small>
        </button>
      ))}
    </div>
  );
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
