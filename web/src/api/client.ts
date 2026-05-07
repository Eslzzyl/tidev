import type {
  Session,
  SessionDetail,
  Message,
  ModelInfo,
  ToolInfo,
  TodoItem,
  CreateSessionRequest,
  CreateSessionResponse,
  SendMessageRequest,
  SendMessageResponse,
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
  DirectoryEntry,
  ListDirResponse,
  ReadFileResponse,
  WriteFileResponse,
  CreateItemRequest,
  CreateItemResponse,
  RenameItemRequest,
  RenameItemResponse,
  RemoveItemResponse,
  ReadBase64Response,
  GitStatusResponse,
  GitBranchResponse,
  GitLogResponse,
  GitMessageResponse,
  GitShowResponse,
  GitFileDiffResponse,
} from "../types/api";

const API_BASE = "/api";

function getAuthToken(): string | null {
  try {
    return localStorage.getItem("web_auth_token");
  } catch {
    return null;
  }
}

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

    if (!response.ok) {
      const error = await response
        .json()
        .catch(() => ({ error: "Unknown error" }));
      throw new Error(error.error || `HTTP ${response.status}`);
    }

    return response.json();
  } catch (error) {
    // Handle network errors (e.g., cannot connect to backend)
    if (error instanceof TypeError && error.message.includes("fetch")) {
      throw new Error(
        "Cannot connect to the server. Please check your network connection and try again.",
      );
    }
    throw error;
  }
}

export const api = {
  // Workspace
  getWorkspace: () => fetchJson<WorkspaceInfo>(`${API_BASE}/workspace`),

  // Sessions
  listSessions: () =>
    fetchJson<{ sessions: Session[] }>(`${API_BASE}/sessions`),

  createSession: (data: CreateSessionRequest) =>
    fetchJson<CreateSessionResponse>(`${API_BASE}/sessions`, {
      method: "POST",
      body: JSON.stringify(data),
    }),

  getSession: (id: string) =>
    fetchJson<SessionDetail>(`${API_BASE}/sessions/${id}`),

  deleteSession: (id: string) =>
    fetchWithAuth(`${API_BASE}/sessions/${id}`, { method: "DELETE" }).then((r) => {
      if (!r.ok) throw new Error(`Failed to delete session: ${r.status}`);
    }),

  // Messages
  listMessages: (sessionId: string) =>
    fetchJson<{ messages: Message[]; todos: TodoItem[] }>(
      `${API_BASE}/sessions/${sessionId}/messages`,
    ),

  sendMessage: (sessionId: string, data: SendMessageRequest) =>
    fetchJson<SendMessageResponse>(
      `${API_BASE}/sessions/${sessionId}/messages`,
      {
        method: "POST",
        body: JSON.stringify(data),
      },
    ),

  abortRequest: (sessionId: string, data: AbortRequest) =>
    fetchJson<{ success: boolean }>(
      `${API_BASE}/sessions/${sessionId}/abort`,
      {
        method: "POST",
        body: JSON.stringify(data),
      },
    ),

  // Models
  listModels: () => fetchJson<{ models: ModelInfo[] }>(`${API_BASE}/models`),

  // Tools
  listTools: () => fetchJson<{ tools: ToolInfo[] }>(`${API_BASE}/tools`),

  // Skills
  listSkills: () => fetchJson<{ skills: SkillInfo[] }>(`${API_BASE}/skills`),

  // Files (@-mention search)
  searchFiles: (query: string) =>
    fetchJson<{ suggestions: FileSuggestion[] }>(
      `${API_BASE}/files/search?q=${encodeURIComponent(query)}`,
    ),

  // Todos
  getTodos: (sessionId: string) =>
    fetchJson<TodosResponse>(`${API_BASE}/sessions/${sessionId}/todos`),

  // Revert / Undo
  revertToMessage: (sessionId: string, messageId: string) =>
    fetchJson<{
      success: boolean;
      reverted_to_message_id: string;
      hidden_message_count: number;
    }>(`${API_BASE}/sessions/${sessionId}/revert`, {
      method: "POST",
      body: JSON.stringify({ message_id: messageId }),
    }),

  // Fork session from a message
  forkSession: (sessionId: string, messageId: string, title?: string) =>
    fetchJson<{
      session_id: string;
      message_count: number;
    }>(`${API_BASE}/sessions/${sessionId}/fork`, {
      method: "POST",
      body: JSON.stringify({ message_id: messageId, title }),
    }),

  // Redo
  redoSession: (sessionId: string) =>
    fetchJson<{ success: boolean; message: string }>(
      `${API_BASE}/sessions/${sessionId}/redo`,
      { method: "POST" },
    ),

  // Compact session context
  compactSession: (sessionId: string) =>
    fetchJson<{ request_id: number }>(
      `${API_BASE}/sessions/${sessionId}/compact`,
      { method: "POST" },
    ),

  // Rename session
  renameSession: (sessionId: string, title: string) =>
    fetchJson<{ success: boolean; title: string }>(
      `${API_BASE}/sessions/${sessionId}/rename`,
      { method: "POST", body: JSON.stringify({ title }) },
    ),

  // Init prompt
  getInitPrompt: () => fetchJson<{ prompt: string }>(`${API_BASE}/init`),

  // Config
  getDefaultModel: () =>
    fetchJson<GetDefaultModelResponse>(`${API_BASE}/config/default-model`),
  setDefaultModel: (data: SetDefaultModelRequest) =>
    fetchJson<SetDefaultModelResponse>(`${API_BASE}/config/default-model`, {
      method: "POST",
      body: JSON.stringify(data),
    }),

  // Agent models
  getAgentModels: () =>
    fetchJson<GetAgentModelsResponse>(`${API_BASE}/config/agent-models`),

  setAgentModel: (data: SetAgentModelRequest) =>
    fetchJson<{ success: boolean }>(`${API_BASE}/config/agent-models`, {
      method: "POST",
      body: JSON.stringify(data),
    }),

  // Providers
  listProviders: () =>
    fetchJson<{ providers: ProviderInfo[] }>(`${API_BASE}/providers`),

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
    fetchJson<ReadFileResponse>(
      `${API_BASE}/fs/read?path=${encodeURIComponent(path)}`,
    ),

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
    fetchJson<ReadBase64Response>(
      `${API_BASE}/fs/read-base64?path=${encodeURIComponent(path)}`,
    ),

  // Terminal
  startTerminal: (cols?: number, rows?: number) =>
    fetchJson<{ session_id: string }>(`${API_BASE}/terminal/start`, {
      method: "POST",
      body: JSON.stringify({ cols, rows }),
    }),

  terminalInput: (sessionId: string, data: string) =>
    fetchWithAuth(`${API_BASE}/terminal/input`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ session_id: sessionId, data }),
    }),

  terminalResize: (sessionId: string, cols: number, rows: number) =>
    fetchWithAuth(`${API_BASE}/terminal/resize`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ session_id: sessionId, cols, rows }),
    }),

  closeTerminal: (sessionId: string) =>
    fetchWithAuth(`${API_BASE}/terminal/${sessionId}`, { method: "DELETE" }),

  // Git
  gitStatus: () => fetchJson<GitStatusResponse>(`${API_BASE}/git/status`),

  gitBranches: (showSubmodules?: boolean) => {
    const params = showSubmodules ? "?show_submodules=true" : "";
    return fetchJson<GitBranchResponse>(`${API_BASE}/git/branches${params}`);
  },

  gitLog: (count = 20, skip = 0) =>
    fetchJson<GitLogResponse>(
      `${API_BASE}/git/history?count=${count}&skip=${skip}`,
    ),

  gitShowCommit: (sha: string) =>
    fetchJson<GitShowResponse>(`${API_BASE}/git/show/${sha}`),

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
    return fetchJson<GitFileDiffResponse>(
      `${API_BASE}/git/diff/file?${params.toString()}`,
    );
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
    fetchJson<GitMessageResponse>(
      `${API_BASE}/git/branch/${encodeURIComponent(name)}`,
      {
        method: "DELETE",
      },
    ),

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
};
