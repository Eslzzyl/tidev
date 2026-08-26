import type { ToolCallEntry, RoundSegment } from "../utils/round";
import type { ProviderErrorData } from "./api";

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
  providerError?: ProviderErrorData;
  userMessageId?: string | null;
  retrying?: {
    attempt: number;
    maxAttempts: number;
    reason: string;
    retryAfterSecs: number | null;
  };
}
