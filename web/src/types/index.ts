export type Feature = "chat" | "files" | "terminal" | "git" | "stats";

export type MessageRole = "system" | "user" | "assistant" | "tool" | "error" | "shell";

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

export interface Message {
  id: string;
  role: MessageRole;
  content: string;
  reasoning?: string;
  tool_calls?: ToolCall[];
  tool_name?: string | null;
  tool_call_id?: string | null;
  created_at?: string;
  completed_at?: string | null;
  streaming?: boolean;
  model_id?: string | null;
  input_tokens?: number | null;
  output_tokens?: number | null;
  total_tokens?: number | null;
}

export interface ToolCall {
  id: string;
  name: string;
  arguments: string;
  thought_signature?: string | null;
}

export interface MessageRecord {
  message: Message;
  app_data: {
    mode?: string | null;
    child_session_id?: string | null;
    snapshot_hash?: string | null;
    patch_files?: string | null;
    file_diffs?: string | null;
  };
}

export interface EventEnvelope {
  cursor: number;
  session_id: string;
  event: Record<string, unknown>;
}

export interface FrontendRequest {
  request_id: string;
  session_id: string;
  kind: {
    ToolApproval?: ToolCallWithViolations[];
  };
}

export interface ToolCallWithViolations {
  tool_call: ToolCall;
  workspace_boundary_violation?: string | null;
  sensitive_file_violation?: string | null;
}

export interface ApprovedTool {
  tool_call: ToolCall;
  rejection: { output: string; attachments: unknown[]; metadata: Record<string, unknown> } | null;
  child_session_id: string | null;
  allow_outside: boolean;
  sensitive_file_approved: boolean;
  user_reason: string | null;
}

export interface StreamMessage {
  key: string;
  requestId: number;
  content: string;
  reasoning: string;
  toolCalls: ToolCall[];
  status: "streaming" | "failed";
  error?: string;
}

export interface Model {
  provider_id: string;
  provider_display_name: string;
  model_id: string;
  model_display_name: string;
  connected: boolean;
  active: boolean;
  thinking_levels: string[];
  thinking_level: string;
}

export interface TodoItem {
  content: string;
  status: string;
}

export interface TerminalShell {
  shell: string;
  configured: boolean;
}

export interface AuthStatus {
  auth_required: boolean;
}
