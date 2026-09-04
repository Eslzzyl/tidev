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
  status: string;
  ended_at: string | null;
  context_summary: string | null;
  context_retained_from: number;
  busy: boolean;
}

export interface SessionDetail extends Session {
  context_summary: string | null;
  context_retained_from: number;
  revert_message_id?: string | null;
}

export interface SessionListCursor {
  updated_at: string;
  session_id: string;
}

export interface SessionListResponse {
  items: Session[];
  next_cursor: SessionListCursor | null;
  workspace_roots: string[];
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
  status: string;
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
  thought_signature?: string | null;
}

export type MessageAttachment =
  | {
      type: "file_reference";
      path: string;
      content: string;
      tool_output: string | null;
      truncated: boolean;
    }
  | {
      type: "directory_reference";
      path: string;
      tree: string;
    }
  | {
      type: "image";
      filename: string;
      mime: string;
      data: number[];
      file_size: number;
    };

export interface FileChangeInfo {
  path: string;
  diff: string | null;
  operation: string;
}

export interface ToolMetadata {
  filepath: string | null;
  diff: string | null;
  truncated: boolean | null;
  exists: boolean | null;
  prior_summary: string | null;
  prior_retained_from: number | null;
  file_changes: FileChangeInfo[];
  exit_code: number | null;
  duration_ms: number | null;
  responses_output_items?: unknown[];
  preserve_full_output?: boolean;
}

export interface ToolExecutionResult {
  output: string;
  attachments: MessageAttachment[];
  metadata: ToolMetadata;
}

export type ThinkingLevelValue = string | Record<string, string | null>;

export interface Message {
  id: string;
  role: "user" | "assistant" | "system" | "tool" | "error";
  content: string;
  attachments: MessageAttachment[];
  reasoning: string;
  tool_calls: ToolCall[];
  tool_call_id: string | null;
  tool_name: string | null;
  metadata: ToolMetadata;
  created_at: string;
  completed_at: string | null;
  streaming: boolean;
  reasoning_started_at: string | null;
  reasoning_completed_at: string | null;
  input_tokens: number | null;
  output_tokens: number | null;
  total_tokens: number | null;
  cache_read_tokens: number | null;
  cache_write_tokens: number | null;
  model_id: string | null;
  tokens_per_second: number | null;
  thinking_level: ThinkingLevelValue | null;
}

export interface MessageRecord {
  message: Message;
  app_data: {
    mode?: string | null;
    child_session_id?: string | null;
    snapshot_hash?: string | null;
    patch_files?: string | null;
    file_diffs?: string | null;
    provider_error?: ProviderErrorData | null;
  };
}

export interface ProviderErrorData {
  message: string;
  retryable: boolean;
  request_id: number;
  user_message_id: string | null;
}

export interface EventEnvelope {
  cursor: number;
  session_id: string;
  event: Record<string, unknown>;
}

export interface ToolCallWithViolations {
  tool_call: ToolCall;
  workspace_boundary_violation?: string | null;
  sensitive_file_violation?: string | null;
}

export interface ApprovedTool {
  tool_call: ToolCall;
  rejection: {
    output: string;
    attachments: unknown[];
    metadata: Record<string, unknown>;
  } | null;
  child_session_id: string | null;
  allow_outside: boolean;
  sensitive_file_approved: boolean;
  user_reason: string | null;
}

export interface FrontendRequest {
  request_id: string;
  session_id: string;
  kind: {
    ToolApproval?: ToolCallWithViolations[];
  };
}

export interface Model {
  provider_id: string;
  provider_display_name: string;
  model_id: string;
  model_display_name: string;
  context_window: number;
  connected: boolean;
  active: boolean;
  supports_vision: boolean;
  thinking_levels: string[];
  thinking_level: string;
}

export interface AuthStatus {
  auth_required: boolean;
}

export interface ModelInfo {
  id: string;
  display_name: string;
  provider_id: string;
  provider_name: string;
  supports_vision: boolean;
  supports_streaming: boolean;
  thinking_supported: boolean;
  thinking_level: string;
  thinking_options: string[];
}

export interface ToolInfo {
  name: string;
  display_name: string;
  description: string;
  permission: string;
}

export interface CreateSessionRequest {
  title?: string;
  workspace_root?: string;
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

export interface GetAgentModelsResponse {
  default_model: GetDefaultModelResponse;
  agent_models: Record<string, string>;
  agent_thinking_levels?: Record<string, string>;
}

export interface SetAgentModelRequest {
  agent_type: string;
  model_str: string;
  thinking_level?: string;
}

export interface GetSubagentConfigResponse {
  enabled: boolean;
}

export interface SetSubagentConfigRequest {
  enabled: boolean;
}

export interface GetMemoryModelResponse {
  role: string;
  model_str: string | null;
}

export interface SetMemoryModelRequest {
  role: string;
  model_str: string;
}

export interface GetModelThinkingLevelResponse {
  provider_id: string;
  model_id: string;
  thinking_level: string | null;
}

export interface SetModelThinkingLevelRequest {
  provider_id: string;
  model_id: string;
  thinking_level: string;
}

export interface WorkspaceInfo {
  workspace_root: string;
  workspace_display: string;
}

export interface WorkspaceContext {
  workspace_root: string;
  workspace_display: string;
  workspace_name: string;
  git_branch: string | null;
}

export interface WorkspaceCompletionResponse {
  directories: string[];
  parent: string | null;
}

export interface CreateSessionResponse {
  session: Session;
}

export interface PromptImageAttachment {
  type: "image";
  filename: string;
  mime: string;
  data: number[];
}

export interface SendMessageRequest {
  content: string;
  thinking_level?: string;
  model_id?: string;
  provider_id?: string;
  mode?: string;
  attachments?: PromptImageAttachment[];
}

export interface SendMessageResponse {
  request_id: number;
}

export interface PromptResponse {
  message_id: string;
  duplicate: boolean;
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
  directory: string;
  location: string;
  is_bundled: boolean;
  companion_files: string[];
  content: string;
  document: string;
}

export interface SkillListResponse {
  skills: SkillInfo[];
}

export interface SkillFileResponse {
  content: string;
}

// Provider types
export interface ProviderModelInfo {
  id: string;
  display_name: string;
  request_model_id: string | null;
  context_window: number;
  max_output_tokens: number;
  api_type: string | null;
  base_url: string | null;
  temperature: number | null;
  supports_images: boolean;
  supports_streaming: boolean;
  supports_parallel_tool_calls: boolean;
}

export interface ProviderInfo {
  id: string;
  display_name: string;
  source: "bundled" | "user";
  can_delete: boolean;
  connected: boolean;
  base_url: string;
  api_type: string | null;
  user_agent: string | null;
  models: ProviderModelInfo[];
}

export interface ConnectProviderRequest {
  api_key: string;
}

export interface ProviderMutationResponse {
  success: boolean;
  connected: boolean | null;
}

export interface CreateModelRequest {
  model_id: string;
  display_name: string;
  context_window: number;
  max_output_tokens: number;
  request_model_id?: string;
  api_type?: string;
  base_url?: string;
  temperature?: number;
  supports_streaming?: boolean;
  supports_images?: boolean;
  supports_parallel_tool_calls?: boolean;
}

export interface CreateProviderRequest {
  provider_id: string;
  display_name: string;
  base_url: string;
  api_type?: string;
  user_agent?: string;
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

export interface GitGraphResponse {
  commits: GitCommitItem[];
  head_sha: string;
  current_branch: string;
}

export interface GitCommitItem {
  sha: string;
  author: string;
  date: string;
  message: string;
  parents?: string[];
  refs?: string[];
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

// ── Stats types ──────────────────────────────────────────────────────────

export interface StatsSummary {
  total_input_tokens: number;
  total_output_tokens: number;
  total_cache_read_tokens: number;
  total_cache_write_tokens: number;
  total_tokens: number;
  total_requests: number;
  cache_hit_rate: number;
  total_sessions: number;
  first_usage_date: string | null;
}

export interface StatsTimeSeriesEntry {
  time_bucket: string;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  request_count: number;
}

export interface StatsTimeSeries {
  granularity: string;
  entries: StatsTimeSeriesEntry[];
  summary: StatsSummary;
}

export interface StatsActivityCell {
  date: string;
  request_count: number;
  total_tokens: number;
  level: number;
}

export interface StatsActivity {
  start_date: string;
  end_date: string;
  total_requests: number;
  total_tokens: number;
  cells: StatsActivityCell[];
}

export interface StatsActiveSessionPoint {
  time_bucket: string;
  active_sessions: number;
}

export interface StatsRhythmCell {
  weekday: number;
  hour: number;
  request_count: number;
  total_tokens: number;
  level: number;
}

export interface StatsModelMixSeries {
  key: string;
  provider_display_name: string;
  model_display_name: string;
  is_other: boolean;
}

export interface StatsModelMixPoint {
  time_bucket: string;
  shares: Record<string, number>;
}

export interface StatsRequestSizeBucket {
  lower_bound: number;
  upper_bound: number | null;
  request_count: number;
  total_tokens: number;
}

export interface StatsInsights {
  granularity: string;
  active_sessions: StatsActiveSessionPoint[];
  rhythm: { cells: StatsRhythmCell[] };
  model_mix: { series: StatsModelMixSeries[]; points: StatsModelMixPoint[] };
  request_size_distribution: StatsRequestSizeBucket[];
}

export interface StatsOverview {
  summary: StatsSummary;
  timeseries: StatsTimeSeries;
  models: { entries: ModelUsageEntry[] };
  providers: { entries: ProviderUsageEntry[] };
  sessions: { entries: SessionUsageEntry[]; total: number };
}

export interface ModelUsageEntry {
  provider_id: string;
  provider_display_name: string;
  model_id: string;
  model_display_name: string;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  request_count: number;
}

export interface ProviderUsageEntry {
  provider_id: string;
  provider_display_name: string;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  request_count: number;
}

export interface SessionUsageEntry {
  session_id: string;
  title: string;
  provider_id: string;
  model_id: string;
  model_display_name: string;
  message_count: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  created_at: string;
  updated_at: string;
}

export type McpServerStatus = "connected" | "connecting" | "disconnected" | "disabled" | "failed";

export type McpServerConfig =
  | {
      type: "stdio";
      command: string;
      args?: string[];
      cwd?: string | null;
      env?: Record<string, string>;
      disabled?: boolean;
    }
  | {
      type: "http";
      url: string;
      headers?: Record<string, string>;
      disabled?: boolean;
    }
  | {
      type: "sse";
      url: string;
      headers?: Record<string, string>;
      disabled?: boolean;
    };

export interface McpToolSummary {
  name: string;
  description: string;
  parameters: Record<string, unknown>;
}

export interface McpServerInfo {
  name: string;
  kind: string;
  status: McpServerStatus;
  error?: string | null;
  disabled?: boolean;
  config?: McpServerConfig | null;
  tools: McpToolSummary[];
}

export interface UpsertMcpServerRequest {
  name: string;
  config: McpServerConfig;
  original_name?: string;
}
