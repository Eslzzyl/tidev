import { Search, FolderTree, RotateCw, Plus, File, Folder, GitBranch } from "lucide-react";
import { useState, useEffect, useCallback, useRef } from "react";
import { FileTree } from "./FileTree";
import { CodeViewer } from "./CodeViewer";
import { useFileStore } from "../../stores/useFileStore";
import { useGitFileStore } from "../../stores/useGitFileStore";
import { api } from "../../api/client";
import { CreateItemDialog } from "../ui/CreateItemDialog";

const SEARCH_CACHE_SIZE = 50;

export function FilesView() {
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<
    { path: string; display: string }[]
  >([]);
  const [isSearching, setIsSearching] = useState(false);
  const [showCreateMenu, setShowCreateMenu] = useState(false);
  const [createType, setCreateType] = useState<"file" | "directory" | null>(
    null,
  );
  const rootLoaded = useFileStore((s) => s.rootLoaded);
  const loadRoot = useFileStore((s) => s.loadRoot);
  const rootLoading = useFileStore((s) => s.rootLoading);
  const createFile = useFileStore((s) => s.createFile);
  const gitBranch = useGitFileStore((s) => s.branch);
  const gitRefresh = useGitFileStore((s) => s.refresh);

  // Search cache: query -> results
  const searchCacheRef = useRef<Record<string, { path: string; display: string }[]>>({});

  // Load root on mount
  useEffect(() => {
    if (!rootLoaded && !rootLoading) {
      loadRoot();
    }
  }, [rootLoaded, rootLoading, loadRoot]);

  // Debounced search with cache
  useEffect(() => {
    if (!searchQuery.trim()) {
      setSearchResults([]);
      return;
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
        const result = await api.searchFiles(searchQuery);
        const mapped = result.suggestions
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

  return (
    <div className="flex h-full">
      {/* Left: File tree panel */}
      <div className="flex w-64 flex-col border-r border-neutral-200 dark:border-neutral-800">
        {/* Search bar */}
        <div className="border-b border-neutral-200 p-2 dark:border-neutral-800">
          <div className="relative">
            <Search className="absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-neutral-400" />
            <input
              type="text"
              placeholder="Search files..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="w-full rounded border border-neutral-200 bg-white py-1 pl-7 pr-2 text-xs outline-none focus:border-neutral-400 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:focus:border-neutral-500"
            />
          </div>
        </div>

        {/* Search results or file tree */}
        <div className="flex-1 overflow-auto">
          {searchQuery.trim() ? (
            <div className="p-2">
              {isSearching ? (
                <p className="py-4 text-center text-xs text-neutral-400">
                  Searching...
                </p>
              ) : searchResults.length > 0 ? (
                <div className="space-y-0.5">
                  {searchResults.map((r) => (
                    <SearchResultItem key={r.path} item={r} />
                  ))}
                </div>
              ) : (
                <p className="py-4 text-center text-xs text-neutral-400">
                  No results
                </p>
              )}
            </div>
          ) : (
            <FileTree />
          )}
        </div>

        {/* Footer with new file/dir buttons and git branch */}
        <div className="flex items-center justify-between border-t border-neutral-200 px-2 py-1.5 dark:border-neutral-800">
          <div className="flex items-center gap-1">
            <span className="text-[10px] text-neutral-400">Files</span>
            {/* New file button */}
            <button
              onClick={() => setCreateType("file")}
              className="rounded p-1 text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800"
              aria-label="New file"
              title="New file"
            >
              <File className="h-3 w-3" />
            </button>
            {/* New directory button */}
            <button
              onClick={() => setCreateType("directory")}
              className="rounded p-1 text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800"
              aria-label="New directory"
              title="New directory"
            >
              <Folder className="h-3 w-3" />
            </button>
            {/* Git branch */}
            {gitBranch && (
              <span className="ml-1 flex items-center gap-0.5 rounded bg-neutral-100 px-1 py-0.5 text-[10px] text-neutral-500 dark:bg-neutral-800 dark:text-neutral-400">
                <GitBranch className="h-2.5 w-2.5" />
                {gitBranch}
              </span>
            )}
          </div>
          <button
            onClick={handleRefresh}
            className="rounded p-1 text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800"
            aria-label="Refresh file tree"
            title="Refresh"
          >
            <RotateCw className="h-3 w-3" />
          </button>
        </div>
      </div>

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

function SearchResultItem({
  item,
}: {
  item: { path: string; display: string };
}) {
  const openFile = useFileStore((s) => s.openFile);

  return (
    <button
      onClick={() => openFile(item.path)}
      className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs hover:bg-neutral-100 dark:hover:bg-neutral-800"
    >
      <span className="truncate text-neutral-700 dark:text-neutral-300">
        {item.display}
      </span>
    </button>
  );
}
