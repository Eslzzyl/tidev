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
  DirectoryEntry,
  ListDirResponse,
  ReadFileResponse,
  GitStatusResponse,
  GitBranchResponse,
  GitLogResponse,
  GitMessageResponse,
} from "../types/api";

const API_BASE = "/api";

async function fetchJson<T>(url: string, options?: RequestInit): Promise<T> {
  try {
    const response = await fetch(url, {
      ...options,
      headers: {
        "Content-Type": "application/json",
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
    fetch(`${API_BASE}/sessions/${id}`, { method: "DELETE" }).then((r) => {
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
    fetch(`${API_BASE}/sessions/${sessionId}/abort`, {
      method: "POST",
      body: JSON.stringify(data),
    }),

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
  getDefaultModel: () => fetchJson<GetDefaultModelResponse>(`${API_BASE}/config/default-model`),
  setDefaultModel: (data: SetDefaultModelRequest) =>
    fetchJson<SetDefaultModelResponse>(`${API_BASE}/config/default-model`, {
      method: "POST",
      body: JSON.stringify(data),
    }),

  // Providers
  listProviders: () =>
    fetchJson<{ providers: ProviderInfo[] }>(`${API_BASE}/providers`),

  connectProvider: (id: string, data: ConnectProviderRequest) =>
    fetch(`${API_BASE}/providers/${encodeURIComponent(id)}/connect`, {
      method: "POST",
      body: JSON.stringify(data),
    }).then((r) => {
      if (!r.ok) throw new Error(`Failed to connect provider: ${r.status}`);
    }),

  disconnectProvider: (id: string) =>
    fetch(`${API_BASE}/providers/${encodeURIComponent(id)}/connect`, {
      method: "DELETE",
    }).then((r) => {
      if (!r.ok) throw new Error(`Failed to disconnect provider: ${r.status}`);
    }),

  createProvider: (data: CreateProviderRequest) =>
    fetch(`${API_BASE}/providers`, {
      method: "POST",
      body: JSON.stringify(data),
    }).then((r) => {
      if (!r.ok) throw new Error(`Failed to create provider: ${r.status}`);
    }),

  deleteProvider: (id: string) =>
    fetch(`${API_BASE}/providers/${encodeURIComponent(id)}`, {
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

  // Terminal
  startTerminal: () =>
    fetchJson<{ session_id: string }>(`${API_BASE}/terminal/start`, {
      method: "POST",
    }),

  terminalInput: (sessionId: string, data: string) =>
    fetch(`${API_BASE}/terminal/input`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ session_id: sessionId, data }),
    }),

  closeTerminal: (sessionId: string) =>
    fetch(`${API_BASE}/terminal/${sessionId}`, { method: "DELETE" }),

  // Git
  gitStatus: () =>
    fetchJson<GitStatusResponse>(`${API_BASE}/git/status`),

  gitBranches: () =>
    fetchJson<GitBranchResponse>(`${API_BASE}/git/branches`),

  gitLog: (count = 20) =>
    fetchJson<GitLogResponse>(`${API_BASE}/git/history?count=${count}`),

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
};
