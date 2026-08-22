export interface DiffSyntaxHunkInput {
  id: string;
  leftLines: string[];
  rightLines: string[];
}

export interface DiffSyntaxHunkResult {
  id: string;
  leftHtml: string[];
  rightHtml: string[];
}

interface DiffSyntaxWorkerResponse {
  type: "result";
  requestId: string;
  hunks: DiffSyntaxHunkResult[];
}

let worker: Worker | null = null;
let requestSequence = 0;
const pending = new Map<
  string,
  {
    resolve: (value: DiffSyntaxHunkResult[]) => void;
    reject: (reason: unknown) => void;
  }
>();

function getWorker(): Worker | null {
  if (typeof Worker === "undefined") return null;
  if (worker) return worker;

  worker = new Worker(new URL("../workers/diffSyntaxWorker.ts", import.meta.url), {
    type: "module",
  });
  worker.onmessage = (event: MessageEvent<DiffSyntaxWorkerResponse>) => {
    if (event.data.type !== "result") return;
    const request = pending.get(event.data.requestId);
    if (!request) return;
    pending.delete(event.data.requestId);
    request.resolve(event.data.hunks);
  };
  worker.onerror = (event) => {
    const error = event.error ?? new Error("Diff syntax worker failed");
    for (const request of pending.values()) request.reject(error);
    pending.clear();
  };
  return worker;
}

export function highlightDiffHunks(
  language: string,
  hunks: DiffSyntaxHunkInput[],
): Promise<DiffSyntaxHunkResult[]> {
  const syntaxWorker = getWorker();
  if (!syntaxWorker) return Promise.resolve([]);

  const requestId = String(++requestSequence);
  return new Promise((resolve, reject) => {
    pending.set(requestId, { resolve, reject });
    syntaxWorker.postMessage({
      type: "highlight",
      requestId,
      language,
      hunks,
    });
  });
}
