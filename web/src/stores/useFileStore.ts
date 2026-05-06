import { create } from "zustand";
import type { DirectoryEntry } from "../types/api";
import { api } from "../api/client";
import { toast } from "./useToastStore";

export interface TreeNode {
  name: string;
  path: string;
  isDirectory: boolean;
  isSymlink: boolean;
  size: number | null;
  modified: string | null;
  expanded: boolean;
  loading: boolean;
  children: TreeNode[];
}

export interface OpenFile {
  path: string;
  content: string;
  language: string | null;
  isDirty: boolean;
  originalContent: string;
}

export interface FileStore {
  rootPath: string;
  rootChildren: TreeNode[];
  rootLoaded: boolean;
  rootLoading: boolean;
  error: string | null;
  selectedPath: string | null;

  /** All currently open files (tabs) */
  openFiles: OpenFile[];
  /** The path of the currently active file tab */
  activeFilePath: string | null;
  /** Whether a save is in progress */
  isSaving: boolean;

  loadRoot: () => Promise<void>;
  toggleExpand: (path: string) => Promise<void>;
  selectFile: (path: string) => void;
  /** Open a file (add to tabs, or switch to existing tab) */
  openFile: (path: string) => Promise<void>;
  /** Close a file tab */
  closeFile: (path: string) => void;
  /** Switch to a specific tab */
  setActiveFile: (path: string) => void;
  setRootPath: (path: string) => void;
  /** Update the content of a specific file (tracks dirty state) */
  updateFileContent: (path: string, content: string) => void;
  /** Save the currently active file */
  saveFile: () => Promise<void>;
  /** Create a new file or directory */
  createFile: (path: string, type: "file" | "directory") => Promise<void>;
  /** Rename/move a file or directory */
  renameFile: (path: string, newPath: string) => Promise<void>;
  /** Delete a file or empty directory */
  deleteFile: (path: string) => Promise<void>;
  /** Refresh the file tree root */
  refreshTree: () => Promise<void>;
}

function buildTreeNodes(entries: DirectoryEntry[]): TreeNode[] {
  return entries.map((e) => ({
    name: e.name,
    path: e.path,
    isDirectory: e.is_directory,
    isSymlink: e.is_symlink,
    size: e.size,
    modified: e.modified,
    expanded: false,
    loading: false,
    children: [],
  }));
}

export const useFileStore = create<FileStore>((set, get) => ({
  rootPath: "",
  rootChildren: [],
  rootLoaded: false,
  rootLoading: false,
  error: null,
  selectedPath: null,
  openFiles: [],
  activeFilePath: null,
  isSaving: false,

  setRootPath: (path) => set({ rootPath: path }),

  loadRoot: async () => {
    set({ rootLoading: true, error: null });
    try {
      const result = await api.listDirectory("");
      set({
        rootChildren: buildTreeNodes(result.entries),
        rootLoaded: true,
        rootLoading: false,
        rootPath: result.directory,
      });
    } catch (err) {
      set({
        rootLoading: false,
        error: err instanceof Error ? err.message : "Failed to load files",
      });
    }
  },

  toggleExpand: async (path: string) => {
    const state = get();

    const updateNode = (nodes: TreeNode[]): TreeNode[] =>
      nodes.map((node) => {
        if (node.path === path) {
          if (node.expanded) {
            return { ...node, expanded: false };
          }
          if (node.children.length === 0 && node.isDirectory) {
            api
              .listDirectory(path)
              .then((result) => {
                const children = buildTreeNodes(result.entries);
                set((s) => ({
                  rootChildren: updateNodeChildren(
                    s.rootChildren,
                    path,
                    children,
                    false,
                  ),
                }));
              })
              .catch(() => {});
            return { ...node, expanded: true, loading: true };
          }
          return { ...node, expanded: true, loading: false };
        }
        if (node.children.length > 0) {
          return { ...node, children: updateNode(node.children) };
        }
        return node;
      });

    set({ rootChildren: updateNode(state.rootChildren) });
  },

  selectFile: (path) => set({ selectedPath: path }),

  openFile: async (path: string) => {
    set({ selectedPath: path });

    // If the file is already open, just switch to it
    const existing = get().openFiles.find((f) => f.path === path);
    if (existing) {
      set({ activeFilePath: path });
      return;
    }

    // Otherwise, load and add to tabs
    try {
      const result = await api.readFile(path);
      const openFile: OpenFile = {
        path,
        content: result.content,
        language: result.language,
        isDirty: false,
        originalContent: result.content,
      };
      set((s) => ({
        openFiles: [...s.openFiles, openFile],
        activeFilePath: path,
      }));
    } catch (err) {
      // Add an error entry as a tab so the user can see the error
      const errorFile: OpenFile = {
        path,
        content: `Error loading file: ${err instanceof Error ? err.message : "Unknown error"}`,
        language: null,
        isDirty: false,
        originalContent: "",
      };
      set((s) => ({
        openFiles: [...s.openFiles, errorFile],
        activeFilePath: path,
      }));
    }
  },

  closeFile: (path: string) => {
    const state = get();
    const idx = state.openFiles.findIndex((f) => f.path === path);
    if (idx < 0) return;

    const newFiles = state.openFiles.filter((f) => f.path !== path);

    // Determine next active file
    let nextActive = state.activeFilePath;
    if (state.activeFilePath === path) {
      if (newFiles.length === 0) {
        nextActive = null;
      } else if (idx < newFiles.length) {
        nextActive = newFiles[idx].path; // same index (next file)
      } else {
        nextActive = newFiles[newFiles.length - 1].path; // last file
      }
    }

    set({
      openFiles: newFiles,
      activeFilePath: nextActive,
      selectedPath:
        nextActive || state.selectedPath === path ? null : state.selectedPath,
    });
  },

  setActiveFile: (path: string) => {
    set({ activeFilePath: path, selectedPath: path });
  },

  updateFileContent: (path: string, content: string) => {
    set((s) => ({
      openFiles: s.openFiles.map((f) =>
        f.path === path
          ? { ...f, content, isDirty: content !== f.originalContent }
          : f,
      ),
    }));
  },

  saveFile: async () => {
    const state = get();
    const activeFile = state.openFiles.find(
      (f) => f.path === state.activeFilePath,
    );
    if (!activeFile) return;

    set({ isSaving: true });
    try {
      await api.writeFile(activeFile.path, activeFile.content);
      set({
        isSaving: false,
        openFiles: state.openFiles.map((f) =>
          f.path === activeFile.path
            ? { ...f, isDirty: false, originalContent: f.content }
            : f,
        ),
      });
    } catch (err) {
      set({ isSaving: false });
      console.error("Failed to save file:", err);
      throw err;
    }
  },

  createFile: async (path, type) => {
    try {
      await api.createItem(path, type);
      toast.success(
        type === "file"
          ? `File created: ${path}`
          : `Directory created: ${path}`,
      );
      get().refreshTree();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Unknown error";
      toast.error(`Failed to create: ${msg}`);
      throw err;
    }
  },

  renameFile: async (path, newPath) => {
    try {
      await api.renameItem(path, newPath);
      toast.success(`Renamed to: ${newPath}`);

      // Update open files if the renamed file was open
      const state = get();
      const wasOpen = state.openFiles.find((f) => f.path === path);
      if (wasOpen) {
        set((s) => ({
          openFiles: s.openFiles.map((f) =>
            f.path === path ? { ...f, path: newPath } : f,
          ),
          activeFilePath:
            s.activeFilePath === path ? newPath : s.activeFilePath,
          selectedPath: s.selectedPath === path ? newPath : s.selectedPath,
        }));
      }

      get().refreshTree();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Unknown error";
      toast.error(`Failed to rename: ${msg}`);
      throw err;
    }
  },

  deleteFile: async (path) => {
    try {
      await api.removeItem(path);
      toast.success(`Deleted: ${path}`);

      // Close tab if the deleted file was open
      const state = get();
      if (state.openFiles.some((f) => f.path === path)) {
        get().closeFile(path);
      }

      get().refreshTree();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Unknown error";
      toast.error(`Failed to delete: ${msg}`);
      throw err;
    }
  },

  refreshTree: async () => {
    try {
      const result = await api.listDirectory("");
      set({
        rootChildren: buildTreeNodes(result.entries),
        rootLoaded: true,
        rootPath: result.directory,
      });
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Unknown error";
      set({ error: msg });
    }
  },
}));

function updateNodeChildren(
  nodes: TreeNode[],
  path: string,
  children: TreeNode[],
  loading: boolean,
): TreeNode[] {
  return nodes.map((node) => {
    if (node.path === path) {
      return { ...node, children, loading, expanded: true };
    }
    if (node.children.length > 0) {
      return {
        ...node,
        children: updateNodeChildren(node.children, path, children, loading),
      };
    }
    return node;
  });
}
