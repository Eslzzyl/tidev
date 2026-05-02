import type {
  Session,
  SessionDetail,
  Message,
  ModelInfo,
  ToolInfo,
  CreateSessionRequest,
  CreateSessionResponse,
  SendMessageRequest,
  SendMessageResponse,
  AbortRequest,
  WorkspaceInfo,
} from '../types/api';

const API_BASE = '/api';

async function fetchJson<T>(url: string, options?: RequestInit): Promise<T> {
  const response = await fetch(url, {
    ...options,
    headers: {
      'Content-Type': 'application/json',
      ...options?.headers,
    },
  });

  if (!response.ok) {
    const error = await response.json().catch(() => ({ error: 'Unknown error' }));
    throw new Error(error.error || `HTTP ${response.status}`);
  }

  return response.json();
}

export const api = {
  // Workspace
  getWorkspace: () => fetchJson<WorkspaceInfo>(`${API_BASE}/workspace`),

  // Sessions
  listSessions: () => fetchJson<{ sessions: Session[] }>(`${API_BASE}/sessions`),

  createSession: (data: CreateSessionRequest) =>
    fetchJson<CreateSessionResponse>(`${API_BASE}/sessions`, {
      method: 'POST',
      body: JSON.stringify(data),
    }),

  getSession: (id: string) => fetchJson<SessionDetail>(`${API_BASE}/sessions/${id}`),

  deleteSession: (id: string) =>
    fetch(`${API_BASE}/sessions/${id}`, { method: 'DELETE' }).then((r) => {
      if (!r.ok) throw new Error(`Failed to delete session: ${r.status}`);
    }),

  // Messages
  listMessages: (sessionId: string) =>
    fetchJson<{ messages: Message[] }>(`${API_BASE}/sessions/${sessionId}/messages`),

  sendMessage: (sessionId: string, data: SendMessageRequest) =>
    fetchJson<SendMessageResponse>(`${API_BASE}/sessions/${sessionId}/messages`, {
      method: 'POST',
      body: JSON.stringify(data),
    }),

  abortRequest: (sessionId: string, data: AbortRequest) =>
    fetch(`${API_BASE}/sessions/${sessionId}/abort`, {
      method: 'POST',
      body: JSON.stringify(data),
    }),

  // Models
  listModels: () => fetchJson<{ models: ModelInfo[] }>(`${API_BASE}/models`),

  // Tools
  listTools: () => fetchJson<{ tools: ToolInfo[] }>(`${API_BASE}/tools`),
};
