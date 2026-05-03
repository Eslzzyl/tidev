import { useEffect, useRef, useState, useCallback } from "react";
import { sseClient } from "../api/sse";
import { useSessionStore } from "../stores/useSessionStore";
import { useUIStore } from "../stores/useUIStore";
import { api } from "../api/client";
import type { AppEvent } from "../types/events";
import type { Round, ToolCallEntry } from "../types/round";
import type { UsageStatsData } from "../stores/useSessionStore";

export function useSSE(sessionId: string | null) {
  const [streamingRound, setStreamingRound] = useState<Round | null>(null);
  const streamingRef = useRef<Round | null>(null);

  const setMessages = useSessionStore((s) => s.setMessages);
  const setStreaming = useUIStore((s) => s.setStreaming);
  const setConnectionStatus = useUIStore((s) => s.setConnectionStatus);
  const currentSessionId = useSessionStore((s) => s.currentSessionId);

  const updateStreamingRound = useCallback(
    (updater: (prev: Round | null) => Round | null) => {
      setStreamingRound((prev) => {
        const next = updater(prev);
        streamingRef.current = next;
        return next;
      });
    },
    [],
  );

  useEffect(() => {
    if (!sessionId) return;

    setConnectionStatus("connecting");

    const createStreamingRound = (): Round | null => {
      const state = useSessionStore.getState();
      const messages = state.messages;
      const lastUserMsg = [...messages]
        .reverse()
        .find((m) => m.role === "user");
      if (!lastUserMsg) return null;

      return {
        id: `streaming-${lastUserMsg.id}`,
        userMessage: lastUserMsg,
        segments: [],
        toolCallMap: {},
        status: "streaming",
      };
    };

    const handleUsageStats = (event: AppEvent) => {
      if (event.type !== "usage_stats") return;
      const stats: UsageStatsData = {
        total_tokens: event.total_tokens,
        input_tokens: event.input_tokens,
        output_tokens: event.output_tokens,
        cache_read_tokens: event.cache_read_tokens,
        cache_write_tokens: event.cache_write_tokens,
        tokens_per_second: event.tokens_per_second,
      };
      useSessionStore.getState().setCurrentUsageStats(stats);
    };

    const handleToolCall = (event: AppEvent) => {
      if (event.type !== "tool_call") return;
      const { tool_call_id, tool_name, arguments: args } = event;

      updateStreamingRound((prev) => {
        const round = prev ?? createStreamingRound();
        if (!round) return null;

        const toolCallMap = { ...round.toolCallMap };
        const existing = toolCallMap[tool_call_id];

        if (existing) {
          if (args && args !== existing.arguments) {
            existing.arguments = args;
          }
          try {
            JSON.parse(existing.arguments);
            existing.argumentsComplete = true;
          } catch {
            // Still streaming
          }
        } else {
          let argsComplete = false;
          try {
            JSON.parse(args || "");
            argsComplete = true;
          } catch {
            // Still streaming
          }

          toolCallMap[tool_call_id] = {
            id: tool_call_id,
            name: tool_name,
            arguments: args || "",
            argumentsComplete: argsComplete,
            resultComplete: false,
          };
        }

        return {
          ...round,
          toolCallMap,
          segments: prev
            ? round.segments
            : [
                ...round.segments,
                { type: "tool_call", toolCallId: tool_call_id },
              ],
        };
      });
    };

    const handleToolResult = (event: AppEvent) => {
      if (event.type !== "tool_result") return;
      const { tool_call_id, output, diff, filepath, rtk_rewritten } = event;

      updateStreamingRound((prev) => {
        if (!prev) return prev;

        const toolCallMap = { ...prev.toolCallMap };
        const entry = toolCallMap[tool_call_id];

        if (entry) {
          toolCallMap[tool_call_id] = {
            ...entry,
            result: {
              output,
              diff: diff || undefined,
              filepath: filepath || undefined,
              rtk_rewritten: rtk_rewritten || false,
              isError: false,
            },
            resultComplete: true,
            argumentsComplete: true,
          };
        }

        return { ...prev, toolCallMap };
      });
    };

    const handleMessageChunk = (event: AppEvent) => {
      if (event.type !== "message_chunk") return;

      updateStreamingRound((prev) => {
        if (prev) {
          const segments = [...prev.segments];
          const lastSeg = segments[segments.length - 1];
          if (lastSeg && lastSeg.type === "text") {
            lastSeg.content += event.content;
          } else {
            segments.push({ type: "text", content: event.content });
          }
          return { ...prev, segments };
        }

        // Create new streaming round with this content
        const state = useSessionStore.getState();
        const messages = state.messages;
        const lastUserMsg = [...messages]
          .reverse()
          .find((m) => m.role === "user");
        if (!lastUserMsg) return null;

        return {
          id: `streaming-${lastUserMsg.id}`,
          userMessage: lastUserMsg,
          segments: [{ type: "text", content: event.content }],
          toolCallMap: {},
          status: "streaming",
        };
      });
    };

    const handleReasoningChunk = (event: AppEvent) => {
      if (event.type !== "reasoning_chunk") return;

      updateStreamingRound((prev) => {
        if (prev) {
          // Append reasoning to the last reasoning segment, or push a new one
          const segments = [...prev.segments];
          const lastSeg = segments[segments.length - 1];
          if (lastSeg && lastSeg.type === "reasoning") {
            lastSeg.content += event.content;
          } else {
            segments.push({ type: "reasoning", content: event.content });
          }
          return { ...prev, segments };
        }

        // Create new streaming round with this reasoning segment
        const state = useSessionStore.getState();
        const messages = state.messages;
        const lastUserMsg = [...messages]
          .reverse()
          .find((m) => m.role === "user");
        if (!lastUserMsg) return null;

        return {
          id: `streaming-${lastUserMsg.id}`,
          userMessage: lastUserMsg,
          segments: [{ type: "reasoning", content: event.content }],
          toolCallMap: {},
          status: "streaming",
        };
      });
    };

    const handleMessageComplete = () => {
      setStreaming(false);

      // Finalize the streaming round
      updateStreamingRound((prev) => {
        if (!prev) return prev;
        return {
          ...prev,
          status: "complete",
          completedAt: new Date().toISOString(),
        };
      });

      // Refresh messages from API
      if (currentSessionId) {
        api.listMessages(currentSessionId).then(({ messages, todos }) => {
          setMessages(messages);
          useSessionStore.getState().setTodos(todos ?? []);
          useSessionStore.getState().setCurrentUsageStats(null);
          streamingRef.current = null;
          setStreamingRound(null);
        });
      } else {
        useSessionStore.getState().setCurrentUsageStats(null);
        streamingRef.current = null;
        setStreamingRound(null);
      }
    };

    const handleErrorEvent = (event: AppEvent) => {
      if (event.type === "error") {
        setStreaming(false);
        setStreamingRound(null);
        streamingRef.current = null;
      } else {
        setConnectionStatus("disconnected");
      }
    };

    const handleAborted = () => {
      setStreaming(false);
      setStreamingRound(null);
      streamingRef.current = null;
    };

    const handleConnected = () => {
      setConnectionStatus("connected");
    };

    const handleMessagesUpdated = () => {
      // Refresh messages from API (e.g. after compaction completes)
      if (currentSessionId) {
        api.listMessages(currentSessionId).then(({ messages, todos }) => {
          setMessages(messages);
          useSessionStore.getState().setTodos(todos ?? []);
        });
      }
    };

    // Register SSE listeners
    sseClient.on("tool.call", handleToolCall);
    sseClient.on("tool.result", handleToolResult);
    sseClient.on("message.chunk", handleMessageChunk);
    sseClient.on("reasoning.chunk", handleReasoningChunk);
    sseClient.on("message.complete", handleMessageComplete);
    sseClient.on("usage.stats", handleUsageStats);
    sseClient.on("error", handleErrorEvent);
    sseClient.on("aborted", handleAborted);
    sseClient.on("connected", handleConnected);
    sseClient.on("messages.updated", handleMessagesUpdated);

    // Connect
    sseClient.connect(sessionId);

    return () => {
      sseClient.off("tool.call", handleToolCall);
      sseClient.off("tool.result", handleToolResult);
      sseClient.off("message.chunk", handleMessageChunk);
      sseClient.off("reasoning.chunk", handleReasoningChunk);
      sseClient.off("message.complete", handleMessageComplete);
      sseClient.off("usage.stats", handleUsageStats);
      sseClient.off("error", handleErrorEvent);
      sseClient.off("aborted", handleAborted);
      sseClient.off("connected", handleConnected);
      sseClient.off("messages.updated", handleMessagesUpdated);
      sseClient.disconnect();
      streamingRef.current = null;
    };
  }, [
    sessionId,
    currentSessionId,
    setMessages,
    setStreaming,
    setConnectionStatus,
    updateStreamingRound,
  ]);

  return streamingRound;
}
