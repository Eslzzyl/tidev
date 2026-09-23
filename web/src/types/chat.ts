import type { ReasoningDisplay, ToolCallEntry, RoundSegment } from "../utils/round";
import type { ProviderErrorData } from "./api";

export type Feature = "chat" | "files" | "terminal" | "git" | "stats";

export interface InstructionNotice {
  sources: string[];
  deferred: boolean;
}

export interface CompactionNotice {
  status: "running" | "complete" | "failed";
  manual: boolean;
  summary: string | null;
  error: string | null;
  modelId: string | null;
  completedAt: string | null;
  afterUserMessageId: string | null;
  beforeRequestId: number | null;
}

export interface StreamMessage {
  key: string;
  requestId: number;
  segments: RoundSegment[];
  reasoningDisplay?: ReasoningDisplay;
  toolCallMap: Record<string, ToolCallEntry>;
  status: "streaming" | "cancelled" | "failed" | "interrupted";
  providerFinished: boolean;
  reasoningStartedAt: string | null;
  reasoningCompletedAt: string | null;
  completedAt?: string | null;
  error?: string;
  providerError?: ProviderErrorData;
  assistantMessageId?: string | null;
  userMessageId?: string | null;
  retrying?: {
    attempt: number;
    maxAttempts: number;
    reason: string;
    retryAfterSecs: number | null;
  };
}
