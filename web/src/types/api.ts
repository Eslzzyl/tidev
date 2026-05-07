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
  revert_message_id?: string | null;
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
  provider_id?: string;
  model_id?: string;
}

export interface SetDefaultModelRequest {
  provider_id: string;
  model_id: string;
}

export interface SetDefaultModelResponse {
  success: boolean;
  provider_id: string;
  model_id: string;
  provider_display_name: string;
  model_display_name: string;
}

export interface GetDefaultModelResponse {
  provider_id: string;
  model_id: string;
  provider_display_name: string;
  model_display_name: string;
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

export interface SkillInfo {
  name: string;
  description: string;
  location: string;
}

// Provider types
export interface ProviderModelInfo {
  id: string;
  display_name: string;
  context_window: number;
  max_output_tokens: number;
  temperature: number;
  supports_images: boolean;
  supports_streaming: boolean;
}

export interface ProviderInfo {
  id: string;
  display_name: string;
  source: "bundled" | "user";
  connected: boolean;
  base_url: string;
  models: ProviderModelInfo[];
}

export interface ConnectProviderRequest {
  api_key: string;
}

export interface CreateModelRequest {
  model_id: string;
  display_name: string;
  context_window: number;
  max_output_tokens: number;
  temperature: number;
  supports_images?: boolean;
}

export interface CreateProviderRequest {
  provider_id: string;
  display_name: string;
  base_url: string;
  api_key: string;
  models: CreateModelRequest[];
}

// Git types
export interface GitStatusResponse {
  branch: string;
  sha: string;
  files: GitStatusFile[];
  ahead: number;
  behind: number;
}

export interface GitStatusFile {
  path: string;
  status: string;
  staged: boolean;
}

export interface GitBranchResponse {
  current: string;
  branches: GitBranchItem[];
}

export interface GitBranchItem {
  name: string;
  current: boolean;
  remote: string | null;
}

export interface GitLogResponse {
  commits: GitCommitItem[];
  has_more: boolean;
}

export interface GitCommitItem {
  sha: string;
  author: string;
  date: string;
  message: string;
}

export interface GitCommitFileInfo {
  path: string;
  status: string; // A, M, D
  additions: number;
  deletions: number;
}

export interface GitShowResponse {
  sha: string;
  author: string;
  date: string;
  message: string;
  files: GitCommitFileInfo[];
  total_additions: number;
  total_deletions: number;
}

export interface GitFileDiffResponse {
  path: string;
  diff: string;
}

export interface GitMessageResponse {
  success: boolean;
  message: string;
}
export interface DirectoryEntry {
  name: string;
  path: string;
  is_directory: boolean;
  is_symlink: boolean;
  size: number | null;
  modified: string | null;
}

export interface ListDirResponse {
  directory: string;
  entries: DirectoryEntry[];
}

export interface ReadFileResponse {
  content: string;
  path: string;
  language: string | null;
  line_count: number;
  size: number;
}

export interface WriteFileResponse {
  path: string;
  size: number;
}

export interface CreateItemRequest {
  path: string;
  type: "file" | "directory";
}

export interface CreateItemResponse {
  path: string;
  type: "file" | "directory";
}

export interface RenameItemRequest {
  path: string;
  new_path: string;
}

export interface RenameItemResponse {
  path: string;
  new_path: string;
}

export interface RemoveItemResponse {
  path: string;
}

export interface ReadBase64Response {
  path: string;
  data: string;
  mime: string;
}
