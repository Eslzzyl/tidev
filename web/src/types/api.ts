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
  status: "added" | "modified" | "deleted";
  additions: number;
  deletions: number;
}

export interface TodoItem {
  content: string;
  status: "pending" | "in_progress" | "completed" | "cancelled";
  priority: "low" | "medium" | "high";
}

export interface TodosResponse {
  todos: TodoItem[];
}

export interface TokenUsage {
  total_tokens?: number;
  input_tokens?: number;
  output_tokens?: number;
  cache_read_tokens?: number;
  cache_write_tokens?: number;
}

export interface ToolCall {
  id: string;
  name: string;
  arguments: string;
}

export interface Message {
  id: string;
  role: "user" | "assistant" | "system" | "tool" | "error" | "shell";
  content: string;
  created_at: string;
  completed_at?: string;
  file_diffs?: FileDiff[];
  todos?: TodoItem[];
  token_usage?: TokenUsage;
  tokens_per_second?: number;
  reasoning?: string;
  tool_call_id?: string;
  tool_name?: string;
  tool_calls?: ToolCall[];
  diff?: string;
  filepath?: string;
  rtk_rewritten?: boolean;
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

export interface FileSuggestion {
  path: string;
  display: string;
  kind: "file" | "directory" | "image";
  matched_indices: number[];
}
