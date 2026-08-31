import { Search, RotateCw, File, Folder, GitBranch, ChevronLeft, PanelLeft } from "lucide-react";
import { useState, useEffect, useCallback, useRef } from "react";
import { FileTree } from "./FileTree";
import { CodeViewer } from "./CodeViewer";
import { useFileStore } from "../../stores/useFileStore";
import { useGitFileStore } from "../../stores/useGitFileStore";
import { api } from "../../api/client";
import { queryClient } from "../../lib/queryClient";
import { CreateItemDialog } from "../ui/CreateItemDialog";
import { Button, IconButton, Input } from "../ui";
import { useTranslation } from "react-i18next";

import { useLocation } from "wouter";
import { routes } from "../../lib/routes";

const SEARCH_CACHE_SIZE = 50;
const MIN_FILETREE_WIDTH = 180;
const MAX_FILETREE_WIDTH = 500;
const DEFAULT_FILETREE_WIDTH = 256;
const COLLAPSED_STRIP_WIDTH = 32;

function loadFileTreeWidth(): number {
  try {
    const saved = localStorage.getItem("filesFileTreeWidth");
    if (saved) {
      return Math.max(MIN_FILETREE_WIDTH, Math.min(MAX_FILETREE_WIDTH, parseInt(saved, 10)));
    }
  } catch {
    // ignore
  }
  return DEFAULT_FILETREE_WIDTH;
}

export function FilesView() {
  const { t } = useTranslation();
  const [, navigate] = useLocation();
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<{ path: string; display: string }[]>([]);
  const [isSearching, setIsSearching] = useState(false);
  const [createType, setCreateType] = useState<"file" | "directory" | null>(null);

  // File tree panel state
  const [fileTreeWidth, setFileTreeWidth] = useState(loadFileTreeWidth);
  const [isMobile, setIsMobile] = useState(
    () => typeof window !== "undefined" && window.innerWidth < 768,
  );
  const [fileTreeOpen, setFileTreeOpen] = useState(!isMobile);
  const [isResizing, setIsResizing] = useState(false);

  const resizeStartRef = useRef({ x: 0, width: 0 });
  const panelRef = useRef<HTMLDivElement>(null);

  const rootLoaded = useFileStore((s) => s.rootLoaded);
  const loadRoot = useFileStore((s) => s.loadRoot);
  const rootLoading = useFileStore((s) => s.rootLoading);
  const openFile = useFileStore((s) => s.openFile);
  const activeFilePath = useFileStore((s) => s.activeFilePath);
  const createFile = useFileStore((s) => s.createFile);
  const gitBranch = useGitFileStore((s) => s.branch);

  // Synchronize URL query `path` with activeFilePath
  const urlPath =
    typeof window !== "undefined" ? new URLSearchParams(window.location.search).get("path") : null;

  useEffect(() => {
    if (urlPath && urlPath !== activeFilePath) {
      void openFile(urlPath);
    }
  }, [urlPath, activeFilePath, openFile]);

  useEffect(() => {
    const currentUrlParam = new URLSearchParams(window.location.search).get("path");
    if (activeFilePath && currentUrlParam !== activeFilePath) {
      navigate(routes.files(activeFilePath), { replace: true });
    } else if (!activeFilePath && currentUrlParam) {
      navigate(routes.files(), { replace: true });
    }
  }, [activeFilePath, navigate]);

  // Search cache: query -> results
  const searchCacheRef = useRef<Record<string, { path: string; display: string }[]>>({});

  // Load root on mount
  useEffect(() => {
    if (!rootLoaded && !rootLoading) {
      loadRoot();
    }
  }, [rootLoaded, rootLoading, loadRoot]);

  // Track viewport width for mobile detection
  useEffect(() => {
    const handleResize = () => {
      const nowMobile = window.innerWidth < 768;
      setIsMobile(nowMobile);
      if (nowMobile) {
        setFileTreeOpen(false);
      }
    };
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, []);

  // Persist file tree width
  useEffect(() => {
    localStorage.setItem("filesFileTreeWidth", String(fileTreeWidth));
  }, [fileTreeWidth]);

  // Debounced search with cache
  useEffect(() => {
    if (!searchQuery.trim()) {
      const id = requestAnimationFrame(() => setSearchResults([]));
      return () => cancelAnimationFrame(id);
    }

    // Check cache first
    const cached = searchCacheRef.current[searchQuery];
    if (cached) {
      setSearchResults(cached);
      return;
    }

    const timer = setTimeout(async () => {
      setIsSearching(true);
      try {
        const result = await queryClient.fetchQuery({
          queryKey: ["files", "search", searchQuery],
          queryFn: () => api.searchFiles(searchQuery),
        });
        const mapped = result.files
          .filter((s) => s.kind === "file")
          .slice(0, 20)
          .map((s) => ({ path: s.path, display: s.display }));

        // Cache with LRU-like eviction
        const cache = searchCacheRef.current;
        const keys = Object.keys(cache);
        if (keys.length >= SEARCH_CACHE_SIZE) {
          delete cache[keys[0]];
        }
        cache[searchQuery] = mapped;

        setSearchResults(mapped);
      } catch {
        // ignore
      } finally {
        setIsSearching(false);
      }
    }, 300);

    return () => clearTimeout(timer);
  }, [searchQuery]);

  const handleRefresh = useCallback(() => {
    loadRoot();
  }, [loadRoot]);

  const handleCreateSubmit = (name: string) => {
    if (!createType) return;
    createFile(name, createType).catch(() => {});
    setCreateType(null);
  };

  const toggleFileTree = useCallback(() => {
    setFileTreeOpen((prev) => !prev);
  }, []);

  // Global resize event handlers (mouse + touch)
  useEffect(() => {
    if (!isResizing) return;

    const handleMove = (e: MouseEvent | TouchEvent) => {
      const clientX = "touches" in e ? e.touches[0].clientX : (e as MouseEvent).clientX;
      const diff = clientX - resizeStartRef.current.x;
      const newWidth = Math.max(
        MIN_FILETREE_WIDTH,
        Math.min(MAX_FILETREE_WIDTH, resizeStartRef.current.width + diff),
      );
      setFileTreeWidth(newWidth);
    };

    const handleEnd = () => {
      setIsResizing(false);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };

    document.addEventListener("mousemove", handleMove);
    document.addEventListener("mouseup", handleEnd);
    document.addEventListener("touchmove", handleMove, { passive: true });
    document.addEventListener("touchend", handleEnd);

    return () => {
      document.removeEventListener("mousemove", handleMove);
      document.removeEventListener("mouseup", handleEnd);
      document.removeEventListener("touchmove", handleMove);
      document.removeEventListener("touchend", handleEnd);
    };
  }, [isResizing]);

  const handleResizeStart = useCallback(
    (e: React.MouseEvent | React.TouchEvent) => {
      e.preventDefault();
      const clientX = "touches" in e ? e.touches[0].clientX : (e as React.MouseEvent).clientX;
      resizeStartRef.current = { x: clientX, width: fileTreeWidth };
      setIsResizing(true);
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
    },
    [fileTreeWidth],
  );

  return (
    <div className="flex h-full">
      {/* Collapsed strip, always visible when the file tree is closed */}
      {!fileTreeOpen && (
        <IconButton
          label={t("Open file browser")}
          size="sm"
          onClick={toggleFileTree}
          className="files-collapsed-button"
          style={{
            width: COLLAPSED_STRIP_WIDTH,
            minWidth: COLLAPSED_STRIP_WIDTH,
          }}
          title={t("Open file browser")}
        >
          <PanelLeft className="h-4 w-4 text-neutral-400" />
        </IconButton>
      )}

      {/* File tree panel */}
      <div
        ref={panelRef}
        className={`flex flex-col border-r border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-950 ${
          fileTreeOpen ? "flex" : "hidden"
        } ${
          // On mobile, when open, overlay the panel
          fileTreeOpen && isMobile ? "fixed inset-y-0 left-0 z-50 shadow-xl" : "relative"
        }`}
        style={fileTreeOpen ? { width: fileTreeWidth, minWidth: MIN_FILETREE_WIDTH } : undefined}
      >
        {/* Header with collapse button */}
        <div className="flex items-center justify-between border-b border-neutral-200 px-2 py-1.5 dark:border-neutral-800">
          <span className="text-xs font-medium text-neutral-500 dark:text-neutral-400">
            {t("Files")}
          </span>
          <div className="flex items-center gap-1">
            {/* New file button */}
            <IconButton
              label={t("New file")}
              size="sm"
              onClick={() => setCreateType("file")}
              title={t("New file")}
            >
              <File className="h-3.5 w-3.5" />
            </IconButton>
            {/* New directory button */}
            <IconButton
              label={t("New directory")}
              size="sm"
              onClick={() => setCreateType("directory")}
              title={t("New directory")}
            >
              <Folder className="h-3.5 w-3.5" />
            </IconButton>
            {/* Refresh */}
            <IconButton
              label={t("Refresh file tree")}
              size="sm"
              onClick={handleRefresh}
              title={t("Refresh")}
            >
              <RotateCw className="h-3.5 w-3.5" />
            </IconButton>
            {/* Collapse button - visible on md+ */}
            <IconButton
              label={t("Close file browser")}
              size="sm"
              onClick={toggleFileTree}
              className="ml-1"
              title={t("Close")}
            >
              <ChevronLeft className="h-3.5 w-3.5" />
            </IconButton>
          </div>
        </div>

        {/* Search bar */}
        <div className="border-b border-neutral-200 p-2 dark:border-neutral-800">
          <div className="relative">
            <Search className="absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-neutral-400" />
            <Input
              type="text"
              placeholder={t("Search files...")}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="file-search-input"
              size="sm"
            />
          </div>
        </div>

        {/* Git branch indicator */}
        {gitBranch && (
          <div className="flex items-center gap-1 border-b border-neutral-200 px-2 py-1 dark:border-neutral-800">
            <span className="flex items-center gap-0.5 rounded bg-neutral-100 px-1.5 py-0.5 text-[10px] text-neutral-500 dark:bg-neutral-800 dark:text-neutral-400">
              <GitBranch className="h-2.5 w-2.5" />
              {gitBranch}
            </span>
          </div>
        )}

        {/* Search results or file tree */}
        <div className="flex-1 overflow-auto">
          {searchQuery.trim() ? (
            <div className="p-2">
              {isSearching ? (
                <p className="py-4 text-center text-xs text-neutral-400">{t("Searching...")}</p>
              ) : searchResults.length > 0 ? (
                <div className="space-y-0.5">
                  {searchResults.map((r) => (
                    <SearchResultItem key={r.path} item={r} />
                  ))}
                </div>
              ) : (
                <p className="py-4 text-center text-xs text-neutral-400">{t("No results")}</p>
              )}
            </div>
          ) : (
            <FileTree />
          )}
        </div>
      </div>

      {/* Mobile overlay backdrop */}
      {fileTreeOpen && isMobile && (
        <Button
          type="button"
          onClick={toggleFileTree}
          className="ui-backdrop-button fixed inset-0 z-40 bg-black/50"
          variant="ghost"
          size="sm"
          aria-label={t("Close file browser")}
        />
      )}

      {/* Resize handle (visible on md+ only when open) */}
      {fileTreeOpen && (
        <div
          onMouseDown={handleResizeStart}
          onTouchStart={handleResizeStart}
          className={`hidden w-1 cursor-col-resize bg-transparent hover:bg-neutral-300 dark:hover:bg-neutral-700 md:block ${
            isResizing ? "bg-neutral-400 dark:bg-neutral-600" : ""
          }`}
          role="separator"
          aria-label={t("Resize file browser")}
        />
      )}

      {/* Right: Code viewer */}
      <div className="flex-1 overflow-hidden">
        <CodeViewer />
      </div>

      {/* Create dialog */}
      {createType && (
        <CreateItemDialog
          parentPath=""
          type={createType}
          onSubmit={handleCreateSubmit}
          onClose={() => setCreateType(null)}
        />
      )}
    </div>
  );
}

function SearchResultItem({ item }: { item: { path: string; display: string } }) {
  const openFile = useFileStore((s) => s.openFile);

  return (
    <Button
      type="button"
      onClick={() => openFile(item.path)}
      className="file-search-result-button"
      variant="ghost"
      size="sm"
    >
      <span className="truncate text-neutral-700 dark:text-neutral-300">{item.display}</span>
    </Button>
  );
}
