import { useCallback, useEffect } from "react";
import {
  ChevronRight,
  ChevronDown,
  File,
  FileText,
  Folder,
  FolderOpen,
  Loader2,
  Code,
  Image,
  Terminal,
} from "lucide-react";
import { useFileStore, type TreeNode } from "../../stores/useFileStore";

const fileIcons: Record<string, React.ReactNode> = {
  rs: <Code className="h-4 w-4 text-orange-500" />,
  ts: <Code className="h-4 w-4 text-blue-500" />,
  tsx: <Code className="h-4 w-4 text-blue-500" />,
  js: <Code className="h-4 w-4 text-yellow-500" />,
  jsx: <Code className="h-4 w-4 text-yellow-500" />,
  py: <Code className="h-4 w-4 text-blue-600" />,
  go: <Code className="h-4 w-4 text-cyan-500" />,
  rb: <Code className="h-4 w-4 text-red-500" />,
  css: <FileText className="h-4 w-4 text-pink-500" />,
  html: <FileText className="h-4 w-4 text-orange-600" />,
  json: <FileText className="h-4 w-4 text-green-600" />,
  md: <FileText className="h-4 w-4 text-blue-400" />,
  toml: <FileText className="h-4 w-4 text-red-400" />,
  yaml: <FileText className="h-4 w-4 text-purple-500" />,
  yml: <FileText className="h-4 w-4 text-purple-500" />,
  sh: <Terminal className="h-4 w-4 text-green-600" />,
  bash: <Terminal className="h-4 w-4 text-green-600" />,
  png: <Image className="h-4 w-4 text-purple-400" />,
  jpg: <Image className="h-4 w-4 text-purple-400" />,
  jpeg: <Image className="h-4 w-4 text-purple-400" />,
  svg: <Image className="h-4 w-4 text-yellow-400" />,
};

function getFileIcon(name: string, isDirectory: boolean): React.ReactNode {
  if (isDirectory) return null; // handled by parent
  const ext = name.includes(".") ? name.split(".").pop()?.toLowerCase() || "" : "";
  return fileIcons[ext] || <File className="h-4 w-4 text-neutral-400" />;
}

export function FileTree() {
  const rootChildren = useFileStore((s) => s.rootChildren);
  const rootLoaded = useFileStore((s) => s.rootLoaded);
  const rootLoading = useFileStore((s) => s.rootLoading);
  const error = useFileStore((s) => s.error);
  const loadRoot = useFileStore((s) => s.loadRoot);
  const selectedPath = useFileStore((s) => s.selectedPath);
  const selectFile = useFileStore((s) => s.selectFile);
  const openFile = useFileStore((s) => s.openFile);
  const toggleExpand = useFileStore((s) => s.toggleExpand);

  useEffect(() => {
    if (!rootLoaded && !rootLoading) {
      loadRoot();
    }
  }, [rootLoaded, rootLoading, loadRoot]);

  const handleNodeClick = useCallback(
    (node: TreeNode) => {
      if (node.isDirectory) {
        toggleExpand(node.path);
      } else {
        selectFile(node.path);
        openFile(node.path);
      }
    },
    [toggleExpand, selectFile, openFile],
  );

  if (rootLoading && !rootLoaded) {
    return (
      <div className="flex items-center justify-center py-8">
        <Loader2 className="h-5 w-5 animate-spin text-neutral-400" />
      </div>
    );
  }

  if (error) {
    return (
      <div className="px-3 py-4 text-center">
        <p className="text-xs text-red-500">{error}</p>
        <button
          onClick={loadRoot}
          className="mt-2 text-xs text-blue-500 hover:underline"
        >
          Retry
        </button>
      </div>
    );
  }

  if (rootChildren.length === 0) {
    return (
      <div className="px-3 py-8 text-center text-xs text-neutral-400">
        No files found
      </div>
    );
  }

  return (
    <div className="select-none text-sm">
      {rootChildren.map((node) => (
        <TreeNodeItem
          key={node.path}
          node={node}
          depth={0}
          selectedPath={selectedPath}
          onNodeClick={handleNodeClick}
          onToggleExpand={toggleExpand}
        />
      ))}
    </div>
  );
}

interface TreeNodeItemProps {
  node: TreeNode;
  depth: number;
  selectedPath: string | null;
  onNodeClick: (node: TreeNode) => void;
  onToggleExpand: (path: string) => Promise<void>;
}

function TreeNodeItem({
  node,
  depth,
  selectedPath,
  onNodeClick,
  onToggleExpand,
}: TreeNodeItemProps) {
  const isSelected = selectedPath === node.path;

  return (
    <div>
      <button
        onClick={() => onNodeClick(node)}
        className={`flex w-full items-center gap-1 px-2 py-1 text-left hover:bg-neutral-100 dark:hover:bg-neutral-800 ${
          isSelected
            ? "bg-blue-50 text-blue-700 dark:bg-blue-950 dark:text-blue-300"
            : "text-neutral-700 dark:text-neutral-300"
        }`}
        style={{ paddingLeft: `${8 + depth * 16}px` }}
        title={node.path}
      >
        {node.isDirectory ? (
          <>
            {node.loading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin text-neutral-400" />
            ) : node.expanded ? (
              <ChevronDown className="h-3.5 w-3.5 shrink-0 text-neutral-400" />
            ) : (
              <ChevronRight className="h-3.5 w-3.5 shrink-0 text-neutral-400" />
            )}
            {node.expanded ? (
              <FolderOpen className="h-4 w-4 shrink-0 text-yellow-500" />
            ) : (
              <Folder className="h-4 w-4 shrink-0 text-yellow-600" />
            )}
          </>
        ) : (
          <>
            <span className="w-3.5 shrink-0" />
            {getFileIcon(node.name, false)}
          </>
        )}
        <span className="truncate text-xs">{node.name}</span>
      </button>

      {/* Render children if expanded */}
      {node.isDirectory && node.expanded && (
        <div>
          {node.children.length > 0 ? (
            node.children.map((child) => (
              <TreeNodeItem
                key={child.path}
                node={child}
                depth={depth + 1}
                selectedPath={selectedPath}
                onNodeClick={onNodeClick}
                onToggleExpand={onToggleExpand}
              />
            ))
          ) : (
            <div
              className="py-1 text-xs text-neutral-400"
              style={{ paddingLeft: `${24 + (depth + 1) * 16}px` }}
            >
              {node.loading ? "Loading..." : "Empty"}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
