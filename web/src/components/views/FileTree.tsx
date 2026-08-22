import { useCallback, useEffect, useRef, useState } from "react";
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
  Copy,
} from "lucide-react";
import { useFileStore, type TreeNode } from "../../stores/useFileStore";
import { useGitFileStore, type GitDisplayStatus } from "../../stores/useGitFileStore";
import { ContextMenu, type ContextMenuItem } from "../ui/ContextMenu";
import { CreateItemDialog } from "../ui/CreateItemDialog";
import { RenameDialog } from "../ui/RenameDialog";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { useTranslation } from "react-i18next";

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
  if (isDirectory) return null;
  const ext = name.includes(".") ? name.split(".").pop()?.toLowerCase() || "" : "";
  return fileIcons[ext] || <File className="h-4 w-4 text-neutral-400" />;
}

/** VS Code-style Git status dot */
function GitStatusDot({ status }: { status?: GitDisplayStatus }) {
  if (!status?.hasChanges) return null;

  let dotClass = "";
  let title = "";

  if (status.isUntracked) {
    dotClass = "bg-neutral-400 dark:bg-neutral-500";
    title = "U";
  } else if (status.hasUnstaged) {
    dotClass = "bg-orange-400 dark:bg-orange-500";
    title = status.rawStatus;
  } else if (status.hasStaged) {
    dotClass = "bg-green-400 dark:bg-green-500";
    title = status.rawStatus;
  }

  return (
    <span
      className={`ml-auto h-2 w-2 shrink-0 rounded-full motion-safe:animate-pulse-soft ${dotClass}`}
      title={title}
    />
  );
}

// Dialog state
type DialogState =
  | { type: "create"; parentPath: string; itemType: "file" | "directory" }
  | { type: "rename"; nodePath: string; nodeName: string }
  | { type: "delete"; nodePath: string; nodeName: string; isDir: boolean }
  | null;

export function FileTree() {
  const { t } = useTranslation();
  const rootChildren = useFileStore((s) => s.rootChildren);
  const rootLoaded = useFileStore((s) => s.rootLoaded);
  const rootLoading = useFileStore((s) => s.rootLoading);
  const error = useFileStore((s) => s.error);
  const loadRoot = useFileStore((s) => s.loadRoot);
  const selectedPath = useFileStore((s) => s.selectedPath);
  const selectFile = useFileStore((s) => s.selectFile);
  const openFile = useFileStore((s) => s.openFile);
  const toggleExpand = useFileStore((s) => s.toggleExpand);
  const createFile = useFileStore((s) => s.createFile);
  const renameFile = useFileStore((s) => s.renameFile);
  const deleteFile = useFileStore((s) => s.deleteFile);
  const gitStatusMap = useGitFileStore((s) => s.displayMap);
  const gitRefresh = useGitFileStore((s) => s.refresh);

  const [contextMenu, setContextMenu] = useState<{
    x: number;
    y: number;
    node: TreeNode | null;
  } | null>(null);
  const [dialog, setDialog] = useState<DialogState>(null);
  const [dragOverPath, setDragOverPath] = useState<string | null>(null);
  const dragNodeRef = useRef<TreeNode | null>(null);

  useEffect(() => {
    if (!rootLoaded && !rootLoading) {
      loadRoot();
    }
  }, [rootLoaded, rootLoading, loadRoot]);

  // Fetch git status when tree is loaded
  useEffect(() => {
    if (rootLoaded) {
      gitRefresh();
    }
  }, [rootLoaded, gitRefresh]);

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

  const handleContextMenu = useCallback((e: React.MouseEvent, node: TreeNode) => {
    e.preventDefault();
    setContextMenu({ x: e.clientX, y: e.clientY, node });
  }, []);

  const buildContextMenuItems = (node: TreeNode): ContextMenuItem[] => {
    const items: ContextMenuItem[] = [];

    items.push({
      label: "Copy Path",
      icon: <Copy className="h-3.5 w-3.5" />,
      onClick: () => {
        navigator.clipboard.writeText(node.path);
      },
    });

    items.push({
      label: "New File",
      icon: <File className="h-3.5 w-3.5" />,
      onClick: () => {
        const parentPath = node.isDirectory ? node.path : getParentPath(node.path);
        setDialog({ type: "create", parentPath, itemType: "file" });
      },
    });

    items.push({
      label: "New Directory",
      icon: <Folder className="h-3.5 w-3.5" />,
      onClick: () => {
        const parentPath = node.isDirectory ? node.path : getParentPath(node.path);
        setDialog({ type: "create", parentPath, itemType: "directory" });
      },
    });

    items.push({
      label: "Rename",
      onClick: () => {
        setDialog({
          type: "rename",
          nodePath: node.path,
          nodeName: node.name,
        });
      },
    });

    items.push({
      label: "Delete",
      danger: true,
      onClick: () => {
        setDialog({
          type: "delete",
          nodePath: node.path,
          nodeName: node.name,
          isDir: node.isDirectory,
        });
      },
    });

    return items;
  };

  const handleCreateSubmit = (name: string) => {
    if (!dialog || dialog.type !== "create") return;
    const fullPath = dialog.parentPath ? `${dialog.parentPath}/${name}` : name;
    createFile(fullPath, dialog.itemType).catch(() => {});
    setDialog(null);
  };

  const handleRenameSubmit = (newName: string) => {
    if (!dialog || dialog.type !== "rename") return;
    const parentPath = getParentPath(dialog.nodePath);
    const newPath = parentPath ? `${parentPath}/${newName}` : newName;
    renameFile(dialog.nodePath, newPath).catch(() => {});
    setDialog(null);
  };

  const handleDeleteConfirm = () => {
    if (!dialog || dialog.type !== "delete") return;
    deleteFile(dialog.nodePath).catch(() => {});
    setDialog(null);
  };

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
        <button onClick={loadRoot} className="mt-2 text-xs text-blue-500 hover:underline">
          {t("Retry")}
        </button>
      </div>
    );
  }

  if (rootChildren.length === 0) {
    return (
      <div className="px-3 py-8 text-center text-xs text-neutral-400">{t("No files found")}</div>
    );
  }

  return (
    <>
      <div className="select-none text-sm" onContextMenu={(e) => e.preventDefault()}>
        {rootChildren.map((node) => (
          <TreeNodeItem
            key={node.path}
            node={node}
            depth={0}
            selectedPath={selectedPath}
            onNodeClick={handleNodeClick}
            onToggleExpand={toggleExpand}
            onContextMenu={handleContextMenu}
            gitDisplayMap={gitStatusMap}
            dragOverPath={dragOverPath}
            onDragStart={(n) => {
              dragNodeRef.current = n;
            }}
            onDragOver={(path) => {
              setDragOverPath(path);
            }}
            onDragLeave={() => {
              setDragOverPath(null);
            }}
            onDrop={(targetPath) => {
              const source = dragNodeRef.current;
              setDragOverPath(null);
              dragNodeRef.current = null;
              if (!source || source.path === targetPath) return;
              const fileName = source.path.split("/").pop() || source.name;
              const newPath = targetPath ? `${targetPath}/${fileName}` : fileName;
              if (newPath !== source.path) {
                renameFile(source.path, newPath).catch(() => {});
              }
            }}
          />
        ))}
      </div>

      {/* Context menu */}
      {contextMenu && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          items={buildContextMenuItems(contextMenu.node!)}
          onClose={() => setContextMenu(null)}
        />
      )}

      {/* Create dialog */}
      {dialog?.type === "create" && (
        <CreateItemDialog
          parentPath={dialog.parentPath}
          type={dialog.itemType}
          onSubmit={handleCreateSubmit}
          onClose={() => setDialog(null)}
        />
      )}

      {/* Rename dialog */}
      {dialog?.type === "rename" && (
        <RenameDialog
          currentName={dialog.nodeName}
          onSubmit={handleRenameSubmit}
          onClose={() => setDialog(null)}
        />
      )}

      {/* Delete confirm dialog */}
      {dialog?.type === "delete" && (
        <ConfirmDialog
          title={t("Delete")}
          message={`${t('Are you sure you want to delete "{{name}}"?', { name: dialog.nodeName })}${
            dialog.isDir ? ` ${t("The directory must be empty.")}` : ""
          }`}
          confirmLabel={t("Delete")}
          danger
          onConfirm={handleDeleteConfirm}
          onCancel={() => setDialog(null)}
        />
      )}
    </>
  );
}

interface TreeNodeItemProps {
  node: TreeNode;
  depth: number;
  selectedPath: string | null;
  onNodeClick: (node: TreeNode) => void;
  onToggleExpand: (path: string) => Promise<void>;
  onContextMenu: (e: React.MouseEvent, node: TreeNode) => void;
  gitDisplayMap: Record<string, GitDisplayStatus>;
  dragOverPath: string | null;
  onDragStart: (node: TreeNode) => void;
  onDragOver: (path: string) => void;
  onDragLeave: () => void;
  onDrop: (targetPath: string) => void;
}

function TreeNodeItem({
  node,
  depth,
  selectedPath,
  onNodeClick,
  // oxlint-disable-next-line only-used-in-recursion
  onToggleExpand,
  onContextMenu,
  gitDisplayMap,
  dragOverPath,
  onDragStart,
  onDragOver: onDragOverCb,
  onDragLeave,
  onDrop,
}: TreeNodeItemProps) {
  const { t } = useTranslation();
  const isSelected = selectedPath === node.path;
  const gitDisplay = gitDisplayMap[node.path];
  const isDragOver = dragOverPath === node.path;

  const handleDragStart = useCallback(
    (e: React.DragEvent) => {
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", node.path);
      onDragStart(node);
    },
    [node, onDragStart],
  );

  const handleDragOver = useCallback(
    (e: React.DragEvent) => {
      if (node.isDirectory) {
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        onDragOverCb(node.path);
      }
    },
    [node.isDirectory, node.path, onDragOverCb],
  );

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      if (node.isDirectory) {
        onDrop(node.path);
      }
    },
    [node.isDirectory, node.path, onDrop],
  );

  return (
    <div>
      <button
        onClick={() => onNodeClick(node)}
        onContextMenu={(e) => onContextMenu(e, node)}
        draggable={!node.isDirectory}
        onDragStart={handleDragStart}
        onDragOver={handleDragOver}
        onDragLeave={onDragLeave}
        onDrop={handleDrop}
        className={`flex w-full items-center gap-1 px-2 py-1 text-left hover:bg-neutral-100 dark:hover:bg-neutral-800 ${
          isSelected
            ? "bg-blue-50 text-blue-700 dark:bg-blue-950 dark:text-blue-300"
            : "text-neutral-700 dark:text-neutral-300"
        } ${isDragOver ? "bg-blue-100 dark:bg-blue-900/30" : ""}`}
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

        {/* VS Code-style Git status dot */}
        <GitStatusDot status={gitDisplay} />
      </button>

      {/* Render children if expanded — smooth height transition */}
      {node.isDirectory && (
        <div
          className="motion-safe:transition-all motion-safe:duration-300 motion-safe:ease-smooth grid"
          style={{
            gridTemplateRows: node.expanded ? "1fr" : "0fr",
            opacity: node.expanded ? 1 : 0,
          }}
        >
          <div className="min-h-0 overflow-hidden">
            {node.children.length > 0 ? (
              node.children.map((child) => (
                <div key={child.path}>
                  <TreeNodeItem
                    node={child}
                    depth={depth + 1}
                    selectedPath={selectedPath}
                    onNodeClick={onNodeClick}
                    onToggleExpand={onToggleExpand}
                    onContextMenu={onContextMenu}
                    gitDisplayMap={gitDisplayMap}
                    dragOverPath={dragOverPath}
                    onDragStart={onDragStart}
                    onDragOver={onDragOverCb}
                    onDragLeave={onDragLeave}
                    onDrop={onDrop}
                  />
                </div>
              ))
            ) : (
              <div
                className="py-1 text-xs text-neutral-400"
                style={{ paddingLeft: `${24 + (depth + 1) * 16}px` }}
              >
                {node.loading ? t("Loading...") : t("Empty")}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function getParentPath(path: string): string {
  const idx = path.lastIndexOf("/");
  return idx >= 0 ? path.substring(0, idx) : "";
}
