import type { ToolCallEntry, RoundSegment } from "../utils/round";

export type Feature = "chat" | "files" | "terminal" | "git" | "stats";

export interface StreamMessage {
  key: string;
  requestId: number;
  segments: RoundSegment[];
  toolCallMap: Record<string, ToolCallEntry>;
  status: "streaming" | "failed" | "interrupted";
  providerFinished: boolean;
  reasoningStartedAt: string | null;
  reasoningCompletedAt: string | null;
  error?: string;
}
