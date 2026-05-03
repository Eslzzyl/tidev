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

  // Rename session
  renameSession: (sessionId: string, title: string) =>
    fetchJson<{ success: boolean; title: string }>(
      `${API_BASE}/sessions/${sessionId}/rename`,
      { method: "POST", body: JSON.stringify({ title }) },
    ),

  // Init prompt
  getInitPrompt: () => fetchJson<{ prompt: string }>(`${API_BASE}/init`),
};
