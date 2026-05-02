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

export interface FileDiff {
	path: string;
	file_path?: string;
	status: 'added' | 'modified' | 'deleted';
	additions: number;
	deletions: number;
}

export interface TodoItem {
	content?: string;
	title?: string;
	status: 'pending' | 'in_progress' | 'completed' | 'cancelled';
	priority: 'low' | 'medium' | 'high';
}

export interface TokenUsage {
	total_tokens?: number;
	input_tokens?: number;
	output_tokens?: number;
}

export interface ToolCall {
	id: string;
	name: string;
	arguments: string;
}

export interface Message {
	id: string;
	role: 'user' | 'assistant' | 'system' | 'tool' | 'error' | 'shell';
	content: string;
	created_at: string;
	// Optional metadata fields
	file_diffs?: FileDiff[];
	todos?: TodoItem[];
	token_usage?: TokenUsage;
	// Thinking/reasoning content for assistant messages
	reasoning?: string;
	// Tool call ID for tool role messages
	tool_call_id?: string;
	// Tool name for tool role messages
	tool_name?: string;
	// Tool calls for assistant messages
	tool_calls?: ToolCall[];
	// Unified diff patch for write/edit tool results (from ToolMetadata)
	diff?: string;
	// File path affected by the tool (from ToolMetadata)
	filepath?: string;
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

export interface WorkspaceInfo {
	workspace_root: string;
}

export interface CreateSessionResponse {
	session_id: string;
}

export interface SendMessageRequest {
	content: string;
	thinking_level?: string;
	model_id?: string;
	provider_id?: string;
	mode?: string;
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
	// Workspace
	getWorkspace: () => fetchJson<WorkspaceInfo>(`${API_BASE}/workspace`),

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
