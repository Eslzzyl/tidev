import { create } from "zustand";
import type { DirectoryEntry } from "../types/api";
import { api } from "../api/client";

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

export interface FileStore {
  rootPath: string;
  rootChildren: TreeNode[];
  rootLoaded: boolean;
  rootLoading: boolean;
  error: string | null;
  selectedPath: string | null;
  openFilePath: string | null;
  openFileContent: string | null;
  openFileLanguage: string | null;

  loadRoot: () => Promise<void>;
  toggleExpand: (path: string) => Promise<void>;
  selectFile: (path: string) => void;
  openFile: (path: string) => Promise<void>;
  closeFile: () => void;
  setRootPath: (path: string) => void;
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
  openFilePath: null,
  openFileContent: null,
  openFileLanguage: null,

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

    // Update the node's expanded state and optionally load children
    const updateNode = (nodes: TreeNode[]): TreeNode[] =>
      nodes.map((node) => {
        if (node.path === path) {
          if (node.expanded) {
            return { ...node, expanded: false };
          }
          // Need to expand - load children if not loaded
          if (node.children.length === 0 && node.isDirectory) {
            // Trigger async load - we'll update after
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
    set({ selectedPath: path, openFilePath: path, openFileContent: null });
    try {
      const result = await api.readFile(path);
      set({
        openFileContent: result.content,
        openFileLanguage: result.language,
      });
    } catch (err) {
      set({
        openFileContent: `Error loading file: ${err instanceof Error ? err.message : "Unknown error"}`,
        openFileLanguage: null,
      });
    }
  },

  closeFile: () =>
    set({
      openFilePath: null,
      openFileContent: null,
      openFileLanguage: null,
    }),
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
