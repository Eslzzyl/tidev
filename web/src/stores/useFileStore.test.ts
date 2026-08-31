import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api } from "../api/client";
import { queryClient } from "../lib/queryClient";
import { useFileStore, type OpenFile } from "./useFileStore";

function createFile(path: string, content = `content for ${path}`): OpenFile {
  return {
    path,
    content,
    language: "text",
    isDirty: false,
    originalContent: content,
  };
}

function createReadResult(path: string) {
  return {
    path,
    content: `content for ${path}`,
    language: "text",
    line_count: 1,
    size: path.length,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

function resetStore() {
  useFileStore.setState({
    rootPath: "",
    rootChildren: [],
    rootLoaded: false,
    rootLoading: false,
    error: null,
    selectedPath: null,
    openFiles: [],
    activeFilePath: null,
    isSaving: false,
  });
}

describe("useFileStore file tabs", () => {
  beforeEach(() => {
    queryClient.clear();
    resetStore();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    queryClient.clear();
  });

  it("keeps the latest requested file active when reads finish out of order", async () => {
    const firstRead = deferred<ReturnType<typeof createReadResult>>();
    const secondRead = deferred<ReturnType<typeof createReadResult>>();
    vi.spyOn(api, "readFile").mockImplementation((path) =>
      path === "first.ts" ? firstRead.promise : secondRead.promise,
    );

    const firstOpen = useFileStore.getState().openFile("first.ts");
    const secondOpen = useFileStore.getState().openFile("second.ts");

    secondRead.resolve(createReadResult("second.ts"));
    await secondOpen;
    expect(useFileStore.getState().activeFilePath).toBe("second.ts");

    firstRead.resolve(createReadResult("first.ts"));
    await firstOpen;

    expect(useFileStore.getState().activeFilePath).toBe("second.ts");
    expect(useFileStore.getState().openFiles.map((file) => file.path)).toEqual([
      "first.ts",
      "second.ts",
    ]);
  });

  it("deduplicates repeated opens of a file that is still loading", async () => {
    const read = deferred<ReturnType<typeof createReadResult>>();
    const readFile = vi.spyOn(api, "readFile").mockReturnValue(read.promise);

    const firstOpen = useFileStore.getState().openFile("same.ts");
    const secondOpen = useFileStore.getState().openFile("same.ts");

    expect(readFile).toHaveBeenCalledTimes(1);

    read.resolve(createReadResult("same.ts"));
    await Promise.all([firstOpen, secondOpen]);

    expect(useFileStore.getState().openFiles.map((file) => file.path)).toEqual(["same.ts"]);
    expect(useFileStore.getState().activeFilePath).toBe("same.ts");
  });

  it("keeps the tree selection when closing an inactive tab", () => {
    useFileStore.setState({
      openFiles: [createFile("first.ts"), createFile("second.ts")],
      activeFilePath: "second.ts",
      selectedPath: "second.ts",
    });

    useFileStore.getState().closeFile("first.ts");

    expect(useFileStore.getState().activeFilePath).toBe("second.ts");
    expect(useFileStore.getState().selectedPath).toBe("second.ts");
  });

  it("selects the next tab and tree node when closing the active tab", () => {
    useFileStore.setState({
      openFiles: [createFile("first.ts"), createFile("second.ts")],
      activeFilePath: "first.ts",
      selectedPath: "first.ts",
    });

    useFileStore.getState().closeFile("first.ts");

    expect(useFileStore.getState().activeFilePath).toBe("second.ts");
    expect(useFileStore.getState().selectedPath).toBe("second.ts");
  });

  it("supersedes an older loading request when the user selects another tab", async () => {
    const read = deferred<ReturnType<typeof createReadResult>>();
    vi.spyOn(api, "readFile").mockReturnValue(read.promise);

    const openingFile = useFileStore.getState().openFile("loading.ts");
    useFileStore.setState({ openFiles: [createFile("existing.ts")] });
    useFileStore.getState().setActiveFile("existing.ts");

    read.resolve(createReadResult("loading.ts"));
    await openingFile;

    expect(useFileStore.getState().activeFilePath).toBe("existing.ts");
    expect(useFileStore.getState().openFiles.map((file) => file.path)).toEqual([
      "existing.ts",
      "loading.ts",
    ]);
  });
});
