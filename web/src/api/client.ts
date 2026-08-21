import type {
  Session,
  SessionDetail,
  MessageRecord,
  Model,
  ApprovedTool,
  ToolInfo,
  CreateSessionRequest,
  CreateSessionResponse,
  SendMessageRequest,
  ShellCommandResponse,
  AbortRequest,
  WorkspaceInfo,
  FileSuggestion,
  TodosResponse,
  SkillInfo,
  ProviderInfo,
  ConnectProviderRequest,
  CreateProviderRequest,
  SetDefaultModelRequest,
  SetDefaultModelResponse,
  GetDefaultModelResponse,
  GetAgentModelsResponse,
  SetAgentModelRequest,
  GetMemoryModelResponse,
  SetMemoryModelRequest,
  GetModelThinkingLevelResponse,
  SetModelThinkingLevelRequest,
  ListDirResponse,
  ReadFileResponse,
  WriteFileResponse,
  CreateItemResponse,
  RenameItemResponse,
  RemoveItemResponse,
  ReadBase64Response,
  GitStatusResponse,
  GitBranchResponse,
  GitLogResponse,
  GitGraphResponse,
  GitMessageResponse,
  GitShowResponse,
  GitFileDiffResponse,
  StatsSummary,
  StatsTimeSeries,
  StatsOverview,
  ModelUsageEntry,
  ProviderUsageEntry,
  SessionUsageEntry,
} from "../types/api";
import { getAuthToken, useAuthStore } from "../stores/useAuthStore";

const API_BASE = "/api";

function getAuthHeaders(): Record<string, string> {
  const token = getAuthToken();
  if (token) {
    return { Authorization: `Bearer ${token}` };
  }
  return {};
}

/** Wrapper around fetch that automatically includes auth headers */
function fetchWithAuth(url: string, options?: RequestInit): Promise<Response> {
  return fetch(url, {
    ...options,
    headers: {
      ...getAuthHeaders(),
      ...options?.headers,
    },
  });
}

async function fetchJson<T>(url: string, options?: RequestInit): Promise<T> {
  try {
    const response = await fetch(url, {
      ...options,
      headers: {
        "Content-Type": "application/json",
        ...getAuthHeaders(),
        ...options?.headers,
      },
    });

    if (response.status === 401) {
      // Token is invalid or expired — trigger auth re-entry
      useAuthStore.getState().handleUnauthorized();
      throw new Error("Unauthorized: invalid or missing auth token");
    }

    if (!response.ok) {
      const error = await response.json().catch(() => ({ error: "Unknown error" }));
      throw new Error(error.error || `HTTP ${response.status}`);
    }

    return response.json();
  } catch (error) {
    // Handle network errors (e.g., cannot connect to backend)
    if (error instanceof TypeError && error.message.includes("fetch")) {
      throw new Error(
        "Cannot connect to the server. Please check your network connection and try again.",
        { cause: error },
      );
    }
    throw error;
  }
}

export const api = {
  // Workspace
  getWorkspace: () => fetchJson<WorkspaceInfo>(`${API_BASE}/workspace`),

  // Sessions
  listSessions: (limit = 100) =>
    fetchJson<Session[]>(`${API_BASE}/sessions?limit=${encodeURIComponent(limit)}`),

  createSession: (data: CreateSessionRequest | string) =>
    fetchJson<CreateSessionResponse>(`${API_BASE}/sessions`, {
      method: "POST",
      body: JSON.stringify(typeof data === "string" ? { title: data } : data),
    }),

  getSession: (id: string) => fetchJson<SessionDetail>(`${API_BASE}/sessions/${id}`),

  deleteSession: (id: string) =>
    fetchJson<{ accepted: boolean }>(`${API_BASE}/sessions/${id}`, { method: "DELETE" }),

  // Messages
  listMessages: (sessionId: string) =>
    fetchJson<{ messages: MessageRecord[] }>(`${API_BASE}/sessions/${sessionId}/messages`),

  sendPrompt: (
    sessionId: string,
    content: string,
    mode: "build" | "plan",
    messageId: string,
    thinkingLevel?: string,
  ) =>
    fetchJson<import("../types/api").PromptResponse>(`${API_BASE}/sessions/${sessionId}/prompts`, {
      method: "POST",
      body: JSON.stringify({
        content,
        mode,
        message_id: messageId,
        thinking_level: thinkingLevel,
      }),
    }),

  sendMessage: (sessionId: string, data: SendMessageRequest) =>
    fetchJson<import("../types/api").PromptResponse>(`${API_BASE}/sessions/${sessionId}/prompts`, {
      method: "POST",
      body: JSON.stringify(data),
    }),

  abortRequest: (sessionId: string, data: AbortRequest) =>
    fetchJson<{ accepted: boolean }>(`${API_BASE}/sessions/${sessionId}/cancel`, {
      method: "POST",
      body: JSON.stringify(data.request_id ? data : undefined),
    }),

  sendShellCommand: (sessionId: string, command: string) =>
    fetchJson<ShellCommandResponse>(`${API_BASE}/sessions/${sessionId}/shell`, {
      method: "POST",
      body: JSON.stringify({ command }),
    }),

  // Models
  listModels: () => fetchJson<Model[]>(`${API_BASE}/models`),

  // Tools
  listTools: () => fetchJson<{ tools: ToolInfo[] }>(`${API_BASE}/tools`),

  // Skills
  listSkills: () => fetchJson<{ skills: SkillInfo[] }>(`${API_BASE}/skills`),

  // Files (@-mention search)
  searchFiles: (query: string) =>
    fetchJson<{ files: FileSuggestion[] }>(
      `${API_BASE}/files/search?q=${encodeURIComponent(query)}`,
    ),

  // Todos
  getTodos: (sessionId: string) =>
    fetchJson<TodosResponse>(`${API_BASE}/sessions/${sessionId}/todos`),

  // Revert / Undo
  revertToMessage: (sessionId: string, messageId: string) =>
    fetchJson<{ accepted: boolean }>(`${API_BASE}/sessions/${sessionId}/revert`, {
      method: "POST",
      body: JSON.stringify({ message_id: messageId }),
    }),

  // Fork session from a message
  forkSession: (sessionId: string, messageId: string, title?: string) =>
    fetchJson<Session>(`${API_BASE}/sessions/${sessionId}/fork`, {
      method: "POST",
      body: JSON.stringify({ message_id: messageId, title }),
    }),

  // Redo
  redoSession: (sessionId: string) =>
    fetchJson<{ accepted: boolean }>(`${API_BASE}/sessions/${sessionId}/redo`, {
      method: "POST",
    }),

  // Compact session context
  compactSession: (sessionId: string) =>
    fetchJson<{ accepted: boolean }>(`${API_BASE}/sessions/${sessionId}/compact`, {
      method: "POST",
    }),

  // Rename session
  renameSession: (sessionId: string, title: string) =>
    fetchJson<Session>(`${API_BASE}/sessions/${sessionId}`, {
      method: "PATCH",
      body: JSON.stringify({ title }),
    }),

  // Chat controls
  selectModel: (providerId: string, modelId: string) =>
    fetchJson<Model>(`${API_BASE}/models/select`, {
      method: "POST",
      body: JSON.stringify({ provider_id: providerId, model_id: modelId }),
    }),

  setThinkingLevel: (providerId: string, modelId: string, thinkingLevel: string) =>
    fetchJson<{ accepted: boolean }>(`${API_BASE}/models/thinking-level`, {
      method: "POST",
      body: JSON.stringify({
        provider_id: providerId,
        model_id: modelId,
        thinking_level: thinkingLevel,
      }),
    }),

  respondToRequest: (requestId: string, approvedTools: ApprovedTool[]) =>
    fetchJson<{ accepted: boolean }>(`${API_BASE}/requests/${requestId}/respond`, {
      method: "POST",
      body: JSON.stringify({ approved_tools: approvedTools }),
    }),

  // Init prompt
  getInitPrompt: (args?: string) =>
    fetchJson<{ prompt: string }>(
      args ? `${API_BASE}/init?args=${encodeURIComponent(args)}` : `${API_BASE}/init`,
    ),

  // Config
  getDefaultModel: () => fetchJson<GetDefaultModelResponse>(`${API_BASE}/config/default-model`),
  setDefaultModel: (data: SetDefaultModelRequest) =>
    fetchJson<SetDefaultModelResponse>(`${API_BASE}/config/default-model`, {
      method: "POST",
      body: JSON.stringify(data),
    }),

  // Agent models
  getAgentModels: () => fetchJson<GetAgentModelsResponse>(`${API_BASE}/config/agent-models`),

  setAgentModel: (data: SetAgentModelRequest) =>
    fetchJson<{ success: boolean }>(`${API_BASE}/config/agent-models`, {
      method: "POST",
      body: JSON.stringify(data),
    }),

  // Memory model
  getMemoryModel: () => fetchJson<GetMemoryModelResponse>(`${API_BASE}/config/memory-model`),

  setMemoryModel: (data: SetMemoryModelRequest) =>
    fetchJson<{ success: boolean }>(`${API_BASE}/config/memory-model`, {
      method: "POST",
      body: JSON.stringify(data),
    }),

  // Thinking level preference
  getModelThinkingLevel: (providerId: string, modelId: string) => {
    const params = new URLSearchParams({
      provider_id: providerId,
      model_id: modelId,
    });
    return fetchJson<GetModelThinkingLevelResponse>(
      `${API_BASE}/config/model-thinking-level?${params}`,
    );
  },
  setModelThinkingLevel: (data: SetModelThinkingLevelRequest) =>
    fetchJson<{ success: boolean }>(`${API_BASE}/config/model-thinking-level`, {
      method: "POST",
      body: JSON.stringify(data),
    }),

  // Providers
  listProviders: () => fetchJson<{ providers: ProviderInfo[] }>(`${API_BASE}/providers`),

  connectProvider: (id: string, data: ConnectProviderRequest) =>
    fetchWithAuth(`${API_BASE}/providers/${encodeURIComponent(id)}/connect`, {
      method: "POST",
      body: JSON.stringify(data),
    }).then((r) => {
      if (!r.ok) throw new Error(`Failed to connect provider: ${r.status}`);
    }),

  disconnectProvider: (id: string) =>
    fetchWithAuth(`${API_BASE}/providers/${encodeURIComponent(id)}/connect`, {
      method: "DELETE",
    }).then((r) => {
      if (!r.ok) throw new Error(`Failed to disconnect provider: ${r.status}`);
    }),

  createProvider: (data: CreateProviderRequest) =>
    fetchWithAuth(`${API_BASE}/providers`, {
      method: "POST",
      body: JSON.stringify(data),
    }).then((r) => {
      if (!r.ok) throw new Error(`Failed to create provider: ${r.status}`);
    }),

  deleteProvider: (id: string) =>
    fetchWithAuth(`${API_BASE}/providers/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }).then((r) => {
      if (!r.ok) throw new Error(`Failed to delete provider: ${r.status}`);
    }),

  // Filesystem
  listDirectory: (path?: string) => {
    const params = path ? `?path=${encodeURIComponent(path)}` : "";
    return fetchJson<ListDirResponse>(`${API_BASE}/fs/list${params}`);
  },

  readFile: (path: string) =>
    fetchJson<ReadFileResponse>(`${API_BASE}/fs/read?path=${encodeURIComponent(path)}`),

  writeFile: (path: string, content: string) =>
    fetchJson<WriteFileResponse>(`${API_BASE}/fs/write`, {
      method: "POST",
      body: JSON.stringify({ path, content }),
    }),

  createItem: (path: string, type: "file" | "directory") =>
    fetchJson<CreateItemResponse>(`${API_BASE}/fs/create`, {
      method: "POST",
      body: JSON.stringify({ path, type }),
    }),

  renameItem: (path: string, newPath: string) =>
    fetchJson<RenameItemResponse>(`${API_BASE}/fs/rename`, {
      method: "POST",
      body: JSON.stringify({ path, new_path: newPath }),
    }),

  removeItem: (path: string) =>
    fetchJson<RemoveItemResponse>(`${API_BASE}/fs/remove`, {
      method: "DELETE",
      body: JSON.stringify({ path }),
    }),

  readFileBase64: (path: string) =>
    fetchJson<ReadBase64Response>(`${API_BASE}/fs/read-base64?path=${encodeURIComponent(path)}`),

  // Terminal
  startTerminal: (cols?: number, rows?: number, shell?: string, label?: string) =>
    fetchJson<{ session_id: string }>(`${API_BASE}/terminal/start`, {
      method: "POST",
      body: JSON.stringify({ cols, rows, shell: shell || undefined, label: label || undefined }),
    }),

  listTerminalShells: () =>
    fetchJson<{
      shells: Array<{ path: string; name: string }>;
      default_shell: string;
    }>(`${API_BASE}/terminal/shells`),

  closeTerminal: (sessionId: string) =>
    fetchWithAuth(`${API_BASE}/terminal/${sessionId}`, { method: "DELETE" }),

  listTerminals: () =>
    fetchJson<{ sessions: Array<{ session_id: string; label: string }> }>(
      `${API_BASE}/terminal/list`,
    ),

  renameTerminal: (sessionId: string, label: string) =>
    fetchJson<{ success: boolean }>(`${API_BASE}/terminal/rename`, {
      method: "POST",
      body: JSON.stringify({ session_id: sessionId, label }),
    }),

  // Terminal shell config (server-side persisted)
  getTerminalShellConfig: () => fetchJson<{ shell: string }>(`${API_BASE}/config/terminal-shell`),

  setTerminalShellConfig: (shell: string) =>
    fetchJson<{ success: boolean; shell: string }>(`${API_BASE}/config/terminal-shell`, {
      method: "POST",
      body: JSON.stringify({ shell }),
    }),

  // Git
  gitStatus: () => fetchJson<GitStatusResponse>(`${API_BASE}/git/status`),

  gitBranches: (showSubmodules?: boolean) => {
    const params = showSubmodules ? "?show_submodules=true" : "";
    return fetchJson<GitBranchResponse>(`${API_BASE}/git/branches${params}`);
  },

  gitLog: (count = 20, skip = 0) =>
    fetchJson<GitLogResponse>(`${API_BASE}/git/history?count=${count}&skip=${skip}`),

  gitGraph: (count = 50) => fetchJson<GitGraphResponse>(`${API_BASE}/git/graph?count=${count}`),

  gitShowCommit: (sha: string) => fetchJson<GitShowResponse>(`${API_BASE}/git/show/${sha}`),

  gitShowFileDiff: (sha: string, path: string) => {
    const params = new URLSearchParams({ path });
    return fetchJson<GitFileDiffResponse[]>(
      `${API_BASE}/git/show/${sha}/diff?${params.toString()}`,
    );
  },

  gitShowAllDiffs: (sha: string) =>
    fetchJson<GitFileDiffResponse[]>(`${API_BASE}/git/show/${sha}/diff`),

  gitDiffFile: (path: string, staged?: boolean) => {
    const params = new URLSearchParams({ path });
    if (staged) params.set("staged", "true");
    return fetchJson<GitFileDiffResponse>(`${API_BASE}/git/diff/file?${params.toString()}`);
  },

  gitCommit: (message: string) =>
    fetchJson<GitMessageResponse>(`${API_BASE}/git/commit`, {
      method: "POST",
      body: JSON.stringify({ message }),
    }),

  gitBranchCreate: (name: string, checkout = false) =>
    fetchJson<GitMessageResponse>(`${API_BASE}/git/branch`, {
      method: "POST",
      body: JSON.stringify({ name, checkout }),
    }),

  gitBranchDelete: (name: string) =>
    fetchJson<GitMessageResponse>(`${API_BASE}/git/branch/${encodeURIComponent(name)}`, {
      method: "DELETE",
    }),

  gitPush: (remote?: string, branch?: string, force?: boolean) =>
    fetchJson<GitMessageResponse>(`${API_BASE}/git/push`, {
      method: "POST",
      body: JSON.stringify({ remote, branch, force }),
    }),

  gitPull: (remote?: string, branch?: string) =>
    fetchJson<GitMessageResponse>(`${API_BASE}/git/pull`, {
      method: "POST",
      body: JSON.stringify({ remote, branch }),
    }),

  gitStash: (message?: string) =>
    fetchJson<GitMessageResponse>(`${API_BASE}/git/stash`, {
      method: "POST",
      body: JSON.stringify({ message }),
    }),

  gitStashPop: () =>
    fetchJson<GitMessageResponse>(`${API_BASE}/git/stash/pop`, {
      method: "POST",
    }),

  // ── Stats ──────────────────────────────────────────────────────────

  getStatsOverview: (params?: {
    granularity?: string;
    start?: string;
    end?: string;
    limit?: number;
    offset?: number;
  }) => {
    const searchParams = new URLSearchParams();
    if (params?.granularity) searchParams.set("granularity", params.granularity);
    if (params?.start) searchParams.set("start", params.start);
    if (params?.end) searchParams.set("end", params.end);
    if (params?.limit !== undefined) searchParams.set("limit", String(params.limit));
    if (params?.offset !== undefined) searchParams.set("offset", String(params.offset));
    const qs = searchParams.toString();
    return fetchJson<StatsOverview>(`${API_BASE}/stats/overview${qs ? `?${qs}` : ""}`);
  },

  getStatsSummary: (params?: { start?: string; end?: string }) => {
    const searchParams = new URLSearchParams();
    if (params?.start) searchParams.set("start", params.start);
    if (params?.end) searchParams.set("end", params.end);
    const qs = searchParams.toString();
    return fetchJson<StatsSummary>(`${API_BASE}/stats/summary${qs ? `?${qs}` : ""}`);
  },

  getStatsTimeSeries: (params?: { granularity?: string; start?: string; end?: string }) => {
    const searchParams = new URLSearchParams();
    if (params?.granularity) searchParams.set("granularity", params.granularity);
    if (params?.start) searchParams.set("start", params.start);
    if (params?.end) searchParams.set("end", params.end);
    const qs = searchParams.toString();
    return fetchJson<StatsTimeSeries>(`${API_BASE}/stats/timeseries${qs ? `?${qs}` : ""}`);
  },

  getStatsModels: (params?: { start?: string; end?: string }) => {
    const searchParams = new URLSearchParams();
    if (params?.start) searchParams.set("start", params.start);
    if (params?.end) searchParams.set("end", params.end);
    const qs = searchParams.toString();
    return fetchJson<{ entries: ModelUsageEntry[] }>(
      `${API_BASE}/stats/models${qs ? `?${qs}` : ""}`,
    );
  },

  getStatsProviders: (params?: { start?: string; end?: string }) => {
    const searchParams = new URLSearchParams();
    if (params?.start) searchParams.set("start", params.start);
    if (params?.end) searchParams.set("end", params.end);
    const qs = searchParams.toString();
    return fetchJson<{ entries: ProviderUsageEntry[] }>(
      `${API_BASE}/stats/providers${qs ? `?${qs}` : ""}`,
    );
  },

  getStatsSessions: (params?: {
    limit?: number;
    offset?: number;
    start?: string;
    end?: string;
  }) => {
    const searchParams = new URLSearchParams();
    if (params?.limit !== undefined) searchParams.set("limit", String(params.limit));
    if (params?.offset !== undefined) searchParams.set("offset", String(params.offset));
    if (params?.start) searchParams.set("start", params.start);
    if (params?.end) searchParams.set("end", params.end);
    const qs = searchParams.toString();
    return fetchJson<{ entries: SessionUsageEntry[]; total: number }>(
      `${API_BASE}/stats/sessions${qs ? `?${qs}` : ""}`,
    );
  },

  /** Request server restart (graceful shutdown + re-exec) */
  restartServer: () => {
    return fetch(`${API_BASE}/system/restart`, {
      method: "POST",
      headers: getAuthHeaders(),
    });
  },
};

/**
 * Wait for the server to restart by polling `/health` and detecting
 * when the `boot_id` changes (indicating a new process instance).
 *
 * Handles both fast restarts (<100ms) and slow restarts (several seconds).
 * Each poll has a per-request timeout so graceful shutdown doesn't hang it.
 *
 * Resolves once the new server is confirmed running.
 * Throws after `timeout` ms.
 */
export async function waitForServerRestart(timeout = 60_000): Promise<void> {
  console.log("[restart] Starting waitForServerRestart");

  // 1. Read the current boot_id before restart
  let oldBootId: string | null = null;
  try {
    const res = await fetch(`/health?_=${Date.now()}`);
    console.log("[restart] Pre-restart health status:", res.status);
    if (res.ok) {
      const data = await res.json();
      console.log("[restart] Pre-restart health body:", JSON.stringify(data));
      oldBootId = data.boot_id ?? null;
      console.log("[restart] Pre-restart boot_id:", oldBootId, "type:", typeof oldBootId);
    }
  } catch (err) {
    console.log("[restart] Pre-restart health failed:", err);
  }

  const pollInterval = 200;
  const perRequestTimeout = 1000;
  const start = Date.now();
  let pollCount = 0;

  while (Date.now() - start < timeout) {
    pollCount++;
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), perRequestTimeout);

    try {
      const res = await fetch(`/health?_=${Date.now()}`, { signal: controller.signal });
      console.log(`[restart] Poll #${pollCount} status:`, res.status);

      if (res.ok) {
        const data = await res.json();
        const newBootId: string | null = data.boot_id ?? null;
        console.log(`[restart] Poll #${pollCount} body:`, JSON.stringify(data));
        console.log(`[restart] Poll #${pollCount} boot_id:`, newBootId, "oldBootId:", oldBootId);

        // boot_id changed → new server process is running
        if (newBootId !== null && newBootId !== oldBootId) {
          console.log("[restart] boot_id changed, restart confirmed!");
          return;
        }
        console.log(`[restart] Poll #${pollCount} boot_id same or null, continuing`);
      } else {
        console.log(`[restart] Poll #${pollCount} non-ok response:`, res.status);
      }
    } catch (err) {
      console.log(`[restart] Poll #${pollCount} fetch failed:`, err);
    } finally {
      clearTimeout(timer);
    }

    // Fallback: if we've been polling for 5s and always got 200 with the same
    // boot_id, the restart probably completed but boot_id comparison failed.
    // Force a page reload to get a fresh state.
    if (Date.now() - start > 5000) {
      console.log("[restart] 5s fallback triggered, forcing reload");
      window.location.reload();
      return;
    }

    await new Promise((r) => setTimeout(r, pollInterval));
  }

  console.log("[restart] Timed out waiting for server restart");
  throw new Error("Server did not come back within timeout");
}
