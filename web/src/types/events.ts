export type AppEvent =
  | { type: 'message_chunk'; session_id: string; request_id: number; content: string }
  | { type: 'message_complete'; session_id: string; request_id: number }
  | {
      type: 'tool_call';
      session_id: string;
      request_id: number;
      tool_call_id: string;
      tool_name: string;
      arguments: string;
    }
  | {
      type: 'tool_result';
      session_id: string;
      request_id: number;
      tool_call_id: string;
      output: string;
      diff?: string;
      filepath?: string;
      rtk_rewritten?: boolean;
    }
  | {
      type: 'permission_request';
      session_id: string;
      request_id: number;
      tool_call_id: string;
      tool_name: string;
      arguments: string;
    }
  | { type: 'aborted'; session_id: string; request_id: number }
  | { type: 'error'; session_id: string; request_id: number; message: string }
  | { type: 'heartbeat' };
