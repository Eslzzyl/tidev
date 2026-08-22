export type Feature = "chat" | "files" | "terminal" | "git" | "stats";

export interface StreamMessage {
  key: string;
  requestId: number;
  content: string;
  reasoning: string;
  toolCalls: import("./api").ToolCall[];
  status: "streaming" | "failed" | "interrupted";
  error?: string;
}
