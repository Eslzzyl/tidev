import type {
  ApprovedTool,
  AuthStatus,
  EventEnvelope,
  FrontendRequest,
  MessageRecord,
  Model,
  Session,
  TerminalShell,
  TodoItem,
} from "./types";

const AUTH_TOKEN_KEY = "web_auth_token";

export function getAuthToken(): string | null {
  try {
    return localStorage.getItem(AUTH_TOKEN_KEY);
  } catch {
    return null;
  }
}

export function setAuthToken(token: string): void {
  try {
    if (token) localStorage.setItem(AUTH_TOKEN_KEY, token);
    else localStorage.removeItem(AUTH_TOKEN_KEY);
  } catch {
    // Local storage can be unavailable in restricted browser contexts.
  }
}

async function request<T>(input: RequestInfo, init?: RequestInit): Promise<T> {
  const token = getAuthToken();
  const response = await fetch(input, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...init?.headers,
    },
  });
  const body = (await response.json()) as T | { error?: string };
  if (!response.ok) {
    throw new Error(
      typeof body === "object" && body && "error" in body ? body.error : response.statusText,
    );
  }
  return body as T;
}

export const api = {
  sessions: (limit = 100) => request<Session[]>(`/api/sessions?limit=${limit}`),
  createSession: (title?: string) =>
    request<{ session: Session }>("/api/sessions", {
      method: "POST",
      body: JSON.stringify({ title }),
    }),
  messages: (sessionId: string) =>
    request<{ messages: MessageRecord[] }>(`/api/sessions/${sessionId}/messages`),
  todos: (sessionId: string) => request<{ todos: TodoItem[] }>(`/api/sessions/${sessionId}/todos`),
  updateSession: (sessionId: string, title: string) =>
    request<Session>(`/api/sessions/${sessionId}`, {
      method: "PATCH",
      body: JSON.stringify({ title }),
    }),
  deleteSession: (sessionId: string) =>
    request<{ accepted: boolean }>(`/api/sessions/${sessionId}`, { method: "DELETE" }),
  sendPrompt: (
    sessionId: string,
    content: string,
    mode: "build" | "plan",
    messageId: string,
    thinkingLevel?: string,
  ) =>
    request<{ message_id: string; duplicate: boolean }>(`/api/sessions/${sessionId}/prompts`, {
      method: "POST",
      body: JSON.stringify({
        content,
        mode,
        message_id: messageId,
        thinking_level: thinkingLevel,
      }),
    }),
  cancel: (sessionId: string) =>
    request<{ accepted: boolean }>(`/api/sessions/${sessionId}/cancel`, { method: "POST" }),
  models: () => request<Model[]>("/api/models"),
  selectModel: (providerId: string, modelId: string) =>
    request<Model>("/api/models/select", {
      method: "POST",
      body: JSON.stringify({ provider_id: providerId, model_id: modelId }),
    }),
  setThinkingLevel: (providerId: string, modelId: string, thinkingLevel: string) =>
    request<{ accepted: boolean }>("/api/models/thinking-level", {
      method: "POST",
      body: JSON.stringify({
        provider_id: providerId,
        model_id: modelId,
        thinking_level: thinkingLevel,
      }),
    }),
  terminalShell: () => request<TerminalShell>("/api/config/terminal-shell"),
  setTerminalShell: (shell: string) =>
    request<TerminalShell>("/api/config/terminal-shell", {
      method: "POST",
      body: JSON.stringify({ shell }),
    }),
  authStatus: () => request<AuthStatus>("/api/auth/status"),
  health: () => request<{ status: string; service: string; frontend: string }>("/api/health"),
  verifyAuthToken: (token: string) =>
    request<{ valid: boolean }>("/api/auth/verify", {
      method: "POST",
      body: JSON.stringify({ token }),
    }),
  configureAuthToken: (newToken: string) =>
    request<{ accepted: boolean }>("/api/auth/configure", {
      method: "POST",
      body: JSON.stringify({ new_token: newToken }),
    }),
  respondToRequest: (requestId: string, approvedTools: ApprovedTool[]) =>
    request<{ accepted: boolean }>(`/api/requests/${requestId}/respond`, {
      method: "POST",
      body: JSON.stringify({ approved_tools: approvedTools }),
    }),
};

export function openBackendEvents(
  after: number | null,
  onEvent: (event: EventEnvelope) => void,
  onResync: () => void,
  onError: () => void,
): EventSource {
  const query = new URLSearchParams();
  if (after !== null) query.set("after", String(after));
  const token = getAuthToken();
  if (token) query.set("token", token);
  const source = new EventSource(`/api/events${query.size ? `?${query}` : ""}`);
  source.addEventListener("backend_event", (event) => {
    try {
      onEvent(JSON.parse((event as MessageEvent).data) as EventEnvelope);
    } catch {
      onError();
    }
  });
  source.addEventListener("resync_required", onResync);
  source.onerror = onError;
  return source;
}

export function openFrontendRequests(
  onRequest: (request: FrontendRequest) => void,
  onError: () => void,
): EventSource {
  const query = new URLSearchParams();
  const token = getAuthToken();
  if (token) query.set("token", token);
  const source = new EventSource(`/api/requests${query.size ? `?${query}` : ""}`);
  source.addEventListener("frontend_request", (event) => {
    try {
      onRequest(JSON.parse((event as MessageEvent).data) as FrontendRequest);
    } catch {
      onError();
    }
  });
  source.onerror = onError;
  return source;
}
