export type AppEvent =
  | {
      type: "message_chunk";
      session_id: string;
      request_id: number;
      content: string;
    }
  | {
      type: "reasoning_chunk";
      session_id: string;
      request_id: number;
      content: string;
    }
  | { type: "message_complete"; session_id: string; request_id: number }
  | {
      type: "usage_stats";
      session_id: string;
      request_id: number;
      total_tokens: number;
      input_tokens: number;
      output_tokens: number;
      cache_read_tokens: number;
      cache_write_tokens: number;
      tokens_per_second?: number;
    }
  | {
      type: "tool_call";
      session_id: string;
      request_id: number;
      tool_call_id: string;
      tool_name: string;
      arguments: string;
    }
  | {
      type: "tool_result";
      session_id: string;
      request_id: number;
      tool_call_id: string;
      output: string;
      diff?: string;
      filepath?: string;
      rtk_rewritten?: boolean;
    }
  | {
      type: "permission_request";
      session_id: string;
      request_id: number;
      tool_call_id: string;
      tool_name: string;
      arguments: string;
    }
  | { type: "aborted"; session_id: string; request_id: number }
  | { type: "error"; session_id: string; request_id: number; message: string }
  | { type: "heartbeat" };
