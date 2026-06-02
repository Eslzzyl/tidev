import { useState, useEffect, useRef, memo } from "react";
import {
  Loader2,
  ChevronDown,
  Search,
  FileText,
  Sparkles,
  LayoutTemplate,
  Wrench,
  Clock,
  MessageSquare,
} from "lucide-react";
import type { ToolCallEntry } from "../../types/round";
import type { Message } from "../../types/api";
import {
  useSubagentStore,
  type SubagentStreamBlock,
} from "../../stores/useSubagentStore";
import { api } from "../../api/client";
import { MarkdownRenderer } from "./MarkdownRenderer";
import { ThinkingBlock } from "./ThinkingBlock";
import { ToolCallRow } from "./ToolCallRow";

interface Props {
  entry: ToolCallEntry;
}

interface AgentConfig {
  name: string;
  color: string;
  bg: string;
  icon: React.ComponentType<{ className?: string }>;
}

const AGENT_CONFIG: Record<string, AgentConfig> = {
  explorer: {
    name: "Explorer",
    color: "text-blue-600 dark:text-blue-400",
    bg: "bg-blue-50 dark:bg-blue-950/30 border-blue-200 dark:border-blue-800",
    icon: Search,
  },
  librarian: {
    name: "Librarian",
    color: "text-purple-600 dark:text-purple-400",
    bg: "bg-purple-50 dark:bg-purple-950/30 border-purple-200 dark:border-purple-800",
    icon: FileText,
  },
  oracle: {
    name: "Oracle",
    color: "text-emerald-600 dark:text-emerald-400",
    bg: "bg-emerald-50 dark:bg-emerald-950/30 border-emerald-200 dark:border-emerald-800",
    icon: Sparkles,
  },
  designer: {
    name: "Designer",
    color: "text-pink-600 dark:text-pink-400",
    bg: "bg-pink-50 dark:bg-pink-950/30 border-pink-200 dark:border-pink-800",
    icon: LayoutTemplate,
  },
  fixer: {
    name: "Fixer",
    color: "text-orange-600 dark:text-orange-400",
    bg: "bg-orange-50 dark:bg-orange-950/30 border-orange-200 dark:border-orange-800",
    icon: Wrench,
  },
};

function getAgentConfig(subagentType: string): AgentConfig {
  const key = subagentType.toLowerCase();
  return (
    AGENT_CONFIG[key] ?? {
      name: subagentType.charAt(0).toUpperCase() + subagentType.slice(1),
      color: "text-amber-600 dark:text-amber-400",
      bg: "bg-amber-50 dark:bg-amber-950/30 border-amber-200 dark:border-amber-800",
      icon: Wrench,
    }
  );
}

function parseTaskArgs(entry: ToolCallEntry): {
  subagentType: string;
  description: string;
  prompt: string;
} {
  try {
    const args = JSON.parse(entry.arguments);
    return {
      subagentType: args.subagent_type || "unknown",
      description: args.description || "",
      prompt: args.prompt || "",
    };
  } catch {
    return { subagentType: "unknown", description: "", prompt: "" };
  }
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.floor(ms / 60000)}m ${Math.floor((ms % 60000) / 1000)}s`;
}

// ---------------------------------------------------------------------------
// Streaming block renderer — uses the same ToolCallRow as the final view
// ---------------------------------------------------------------------------

let streamBlockIdCounter = 0;

function StreamingBlock({ block }: { block: SubagentStreamBlock }) {
  switch (block.type) {
    case "reasoning":
      return (
        <div className="mb-2">
          <ThinkingBlock content={block.content ?? ""} defaultExpanded={true} />
        </div>
      );
    case "content":
      return (
        <div className="mb-2 text-sm text-neutral-700 dark:text-neutral-300">
          <MarkdownRenderer content={block.content ?? ""} />
        </div>
      );
    case "tool_call": {
      // Build a partial ToolCallEntry for consistent rendering
      const toolEntry: ToolCallEntry = {
        id: `stream-tc-${++streamBlockIdCounter}`,
        name: block.toolName ?? "tool",
        arguments: block.toolArgs ?? "{}",
        argumentsComplete: true,
        resultComplete: block.complete ?? false,
      };
      return (
        <div className="mb-1">
          <ToolCallRow entry={toolEntry} />
        </div>
      );
    }
    default:
      return null;
  }
}

// ---------------------------------------------------------------------------
// Child session message renderer (fetched after completion)
// ---------------------------------------------------------------------------

function ChildSessionMessages({ messages }: { messages: Message[] }) {
  if (!messages || messages.length === 0) return null;

  const blocks: { key: string; element: React.ReactNode }[] = [];

  for (let i = 0; i < messages.length; i++) {
    const msg = messages[i];

    if (msg.role === "assistant") {
      if (msg.reasoning) {
        blocks.push({
          key: `reasoning-${msg.id}`,
          element: (
            <div className="mb-2">
              <ThinkingBlock content={msg.reasoning} defaultExpanded={true} />
            </div>
          ),
        });
      }

      if (msg.content) {
        blocks.push({
          key: `text-${msg.id}`,
          element: (
            <div className="mb-2 text-sm text-neutral-700 dark:text-neutral-300">
              <MarkdownRenderer content={msg.content} />
            </div>
          ),
        });
      }

      if (msg.tool_calls && msg.tool_calls.length > 0) {
        for (const tc of msg.tool_calls) {
          const resultMsg = messages.find(
            (m) => m.role === "tool" && m.tool_call_id === tc.id,
          );
          const toolEntry: ToolCallEntry = {
            id: tc.id,
            name: tc.name,
            arguments: tc.arguments,
            argumentsComplete: true,
            result: resultMsg
              ? {
                  output: resultMsg.content,
                  diff: resultMsg.diff,
                  filepath: resultMsg.filepath,
                }
              : undefined,
            resultComplete: !!resultMsg,
          };
          blocks.push({
            key: `tool-${tc.id}`,
            element: (
              <div className="mb-1">
                <ToolCallRow entry={toolEntry} defaultExpanded={!!resultMsg} />
              </div>
            ),
          });
        }
      }
    }
  }

  return (
    <div className="space-y-1">
      {blocks.map((b) => (
        <div key={b.key}>{b.element}</div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// SubagentCard
// ---------------------------------------------------------------------------

export const SubagentCard = memo(function SubagentCard({ entry }: Props) {
  const [expanded, setExpanded] = useState(false);
  const [elapsedMs, setElapsedMs] = useState(0);
  const [childMessages, setChildMessages] = useState<Message[] | null>(null);
  const [childMessagesLoading, setChildMessagesLoading] = useState(false);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const startTimeRef = useRef<number | null>(null);
  const hasFetchedRef = useRef(false);
  const didAutoExpandRef = useRef(false);

  const { subagentType, description, prompt } = parseTaskArgs(entry);
  const config = getAgentConfig(subagentType);

  const subagentState = useSubagentStore((s) => s.states[entry.id]);

  const isRunning =
    !entry.resultComplete &&
    entry.argumentsComplete &&
    !subagentState?.completed;
  const isCompleted = entry.resultComplete || subagentState?.completed;
  const childSessionId = subagentState?.childSessionId;
  const Icon = config.icon;

  const hasBlocks = subagentState?.blocks && subagentState.blocks.length > 0;

  // Auto-expand when first streaming block arrives
  useEffect(() => {
    if (hasBlocks && !didAutoExpandRef.current) {
      didAutoExpandRef.current = true;
      setExpanded(true);
    }
  }, [hasBlocks]);

  // Live elapsed timer
  useEffect(() => {
    if (isRunning) {
      startTimeRef.current = Date.now();
      timerRef.current = setInterval(() => {
        setElapsedMs(Date.now() - (startTimeRef.current ?? Date.now()));
      }, 100);
    } else if (isCompleted && startTimeRef.current) {
      setElapsedMs(Date.now() - startTimeRef.current);
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
    }

    return () => {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [isRunning, isCompleted]);

  // Fetch child session messages when expanded + completed
  useEffect(() => {
    if (!(expanded && isCompleted && childSessionId)) return;

    const fetchMessages = async () => {
      if (!childSessionId || hasFetchedRef.current) return;
      hasFetchedRef.current = true;
      setChildMessagesLoading(true);
      try {
        const res = await api.listMessages(childSessionId);
        setChildMessages(res.messages);
      } catch (err) {
        console.error("[SubagentCard] failed to fetch child messages:", err);
      } finally {
        setChildMessagesLoading(false);
      }
    };

    fetchMessages();
  }, [expanded, isCompleted, childSessionId]);

  useEffect(() => {
    hasFetchedRef.current = false;
    didAutoExpandRef.current = false;
    const id = requestAnimationFrame(() => setChildMessages(null));
    return () => cancelAnimationFrame(id);
  }, [entry.id]);

  function handleToggle() {
    setExpanded((prev) => !prev);
  }

  const isExpanded = expanded;

  const showPrompt = prompt && isExpanded;
  const showChildMessages =
    isCompleted && childMessages && childMessages.length > 0;
  const showStreamingBlocks = hasBlocks && !showChildMessages;
  const showLoadingMessages =
    isCompleted && childSessionId && childMessagesLoading;
  const showWaiting = isRunning && !hasBlocks;
  const showSessionSection = showStreamingBlocks || showChildMessages;

  return (
    <div className={`my-2 overflow-hidden rounded-lg border ${config.bg}`}>
      {/* Header */}
      <button
        onClick={handleToggle}
        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-black/5 dark:hover:bg-white/5"
      >
        <Icon className={`h-3.5 w-3.5 flex-shrink-0 ${config.color}`} />

        <div className="flex flex-1 flex-col min-w-0">
          <span className={`text-xs font-medium ${config.color}`}>
            {config.name}
          </span>
          {description && (
            <span className="truncate text-xs text-neutral-500 dark:text-neutral-400">
              {description}
            </span>
          )}
        </div>

        <div className="flex items-center gap-2 flex-shrink-0">
          {isRunning && subagentState?.statusText && (
            <span className="hidden sm:inline-block max-w-[200px] truncate text-[10px] text-neutral-400 dark:text-neutral-500 italic">
              {subagentState.statusText}
            </span>
          )}

          {isRunning && (
            <>
              <Loader2 className="h-3.5 w-3.5 animate-spin text-neutral-400" />
              {elapsedMs > 0 && (
                <span className="text-xs tabular-nums text-neutral-400">
                  {formatDuration(elapsedMs)}
                </span>
              )}
            </>
          )}

          {isCompleted && elapsedMs > 0 && (
            <span className="flex items-center gap-1 text-xs text-neutral-400">
              <Clock className="h-3 w-3" />
              {formatDuration(elapsedMs)}
            </span>
          )}

          <ChevronDown
            className={`h-3.5 w-3.5 text-neutral-400 transition-transform ${
              expanded ? "rotate-180" : ""
            }`}
          />
        </div>
      </button>

      {/* Expanded Content */}
      {isExpanded && (
        <div className="border-t border-inherit">
          <div className="px-3 py-2 space-y-2">
            {showPrompt && (
              <div>
                <div className="mb-1 text-[10px] font-medium uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
                  Task Prompt
                </div>
                <div className="rounded bg-black/5 px-3 py-2 text-xs text-neutral-600 whitespace-pre-wrap break-words dark:bg-white/5 dark:text-neutral-400">
                  {prompt}
                </div>
              </div>
            )}

            {showPrompt && showSessionSection && (
              <div className="border-t border-neutral-200 dark:border-neutral-700" />
            )}

            {showSessionSection && (
              <div className="flex items-center gap-1.5">
                <MessageSquare className="h-3 w-3 text-neutral-400" />
                <span className="text-[10px] font-medium uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
                  Sub-session
                </span>
                {isRunning && (
                  <Loader2 className="h-2.5 w-2.5 animate-spin text-neutral-400" />
                )}
              </div>
            )}

            {showStreamingBlocks &&
              subagentState.blocks.map((block, idx) => (
                <StreamingBlock key={`block-${idx}`} block={block} />
              ))}

            {showLoadingMessages && (
              <div className="flex items-center gap-2 text-xs text-neutral-400 py-2">
                <Loader2 className="h-3 w-3 animate-spin" />
                Loading sub-session content...
              </div>
            )}

            {showWaiting && (
              <div className="flex items-center gap-1.5 text-xs text-neutral-400 dark:text-neutral-500 italic">
                <Loader2 className="h-3 w-3 animate-spin" />
                Waiting for subagent response...
              </div>
            )}

            {showChildMessages && (
              <ChildSessionMessages messages={childMessages} />
            )}
          </div>
        </div>
      )}
    </div>
  );
});
