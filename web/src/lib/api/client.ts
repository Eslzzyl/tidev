const API_BASE = '/api';

export interface Session {
	session_id: string;
	parent_session_id: string | null;
	workspace_root: string;
	provider_id: string;
	provider_display_name: string;
	model_id: string;
	model_display_name: string;
	title: string;
	created_at: string;
	updated_at: string;
}

export interface SessionDetail extends Session {
	context_summary: string | null;
	context_retained_from: number;
}

export interface Message {
	id: string;
	role: 'user' | 'assistant' | 'system' | 'tool' | 'error' | 'shell';
	content: string;
	created_at: string;
}

export interface ModelInfo {
	id: string;
	display_name: string;
	provider_id: string;
	provider_name: string;
	supports_vision: boolean;
	supports_streaming: boolean;
}

export interface ToolInfo {
	name: string;
	display_name: string;
	description: string;
	permission: string;
}

export interface CreateSessionRequest {
	workspace_root: string;
	title?: string;
}

export interface CreateSessionResponse {
	session_id: string;
}

export interface SendMessageRequest {
	content: string;
	thinking_level?: string;
}

export interface SendMessageResponse {
	request_id: number;
}

export interface AbortRequest {
	request_id: number;
}

async function fetchJson<T>(url: string, options?: RequestInit): Promise<T> {
	const response = await fetch(url, {
		...options,
		headers: {
			'Content-Type': 'application/json',
			...options?.headers
		}
	});

	if (!response.ok) {
		const error = await response.json().catch(() => ({ error: 'Unknown error' }));
		throw new Error(error.error || `HTTP ${response.status}`);
	}

	return response.json();
}

export const api = {
	// Sessions
	listSessions: () => fetchJson<{ sessions: Session[] }>(`${API_BASE}/sessions`),

	createSession: (data: CreateSessionRequest) =>
		fetchJson<CreateSessionResponse>(`${API_BASE}/sessions`, {
			method: 'POST',
			body: JSON.stringify(data)
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
			body: JSON.stringify(data)
		}),

	abortRequest: (sessionId: string, data: AbortRequest) =>
		fetch(`${API_BASE}/sessions/${sessionId}/abort`, {
			method: 'POST',
			body: JSON.stringify(data)
		}),

	// Models
	listModels: () => fetchJson<{ models: ModelInfo[] }>(`${API_BASE}/models`),

	// Tools
	listTools: () => fetchJson<{ tools: ToolInfo[] }>(`${API_BASE}/tools`)
};
