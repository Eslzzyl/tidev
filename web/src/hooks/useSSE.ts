import { useEffect, useRef, useState, useCallback } from "react";
import { sseClient } from "../api/sse";
import { usePermissionStore } from "../stores/usePermissionStore";
import { useSessionStore } from "../stores/useSessionStore";
import { useUIStore } from "../stores/useUIStore";
import { api } from "../api/client";
import type { AppEvent } from "../types/events";
import type { Round, ToolCallEntry } from "../types/round";
import type { UsageStatsData } from "../stores/useSessionStore";
import type { Message, ToolCall } from "../types/api";

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
      if (!lastUserMsg) {
        console.log("[SSE] createStreamingRound: no user message found, messages:", messages.length);
        return null;
      }
      console.log("[SSE] createStreamingRound: found user msg id:", lastUserMsg.id.substring(0,20));

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

    const handleShellOutput = (event: AppEvent) => {
      if (event.type !== "shell_output") return;
      const { content, finished, exit_code } = event;

      updateStreamingRound((prev) => {
        if (!prev) return prev;

        // Find all bash tool calls in the round and update their streaming output
        const toolCallMap = { ...prev.toolCallMap };
        let changed = false;

        for (const [id, entry] of Object.entries(toolCallMap)) {
          if (entry.name === "bash") {
            // Parse exit code from content if present (format: "[exit N]\n...")
            let exitCode: number | null = exit_code ?? null;
            let cleanContent = content;

            // If the backend already formatted with [exit N], extract it
            const exitMatch = content.match(/^\[exit\s*(-?\d+)\]\n/);
            if (exitMatch) {
              exitCode = parseInt(exitMatch[1], 10);
              cleanContent = content.slice(exitMatch[0].length);
            }

            toolCallMap[id] = {
              ...entry,
              result: {
                output: cleanContent,
                exitCode,
                isError: exitCode !== null && exitCode !== 0,
              },
              resultComplete: finished,
              argumentsComplete: true,
            };
            changed = true;
          }
        }

        return changed ? { ...prev, toolCallMap } : prev;
      });
    };

    const handleMessageChunk = (event: AppEvent) => {
      if (event.type !== "message_chunk") return;

      updateStreamingRound((prev) => {
        if (prev) {
          const segments = [...prev.segments];
          const lastIdx = segments.length - 1;
          const lastSeg = segments[lastIdx];
          if (lastSeg && lastSeg.type === "text") {
            // Create a new segment object (immutable) to avoid mutating prev state
            segments[lastIdx] = {
              ...lastSeg,
              content: lastSeg.content + event.content,
            };
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
        if (!lastUserMsg) {
          console.log("[SSE] handleMessageChunk: no user msg found in store, messages:", messages.length);
          return null;
        }
        console.log("[SSE] handleMessageChunk: creating streaming round with user msg:", lastUserMsg.id.substring(0,20));

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
          const lastIdx = segments.length - 1;
          const lastSeg = segments[lastIdx];
          if (lastSeg && lastSeg.type === "reasoning") {
            // Create a new segment object (immutable) to avoid mutating prev state
            segments[lastIdx] = {
              ...lastSeg,
              content: lastSeg.content + event.content,
            };
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
        if (!lastUserMsg) {
          console.log("[SSE] handleReasoningChunk: no user msg found, messages:", messages.length);
          return null;
        }
        console.log("[SSE] handleReasoningChunk: creating streaming round, user msg:", lastUserMsg.id.substring(0,20));

        return {
          id: `streaming-${lastUserMsg.id}`,
          userMessage: lastUserMsg,
          segments: [{ type: "reasoning", content: event.content }],
          toolCallMap: {},
          status: "streaming",
        };      });
    };

    const handleMessageComplete = () => {
      console.log("[SSE] message.complete fired, currentSessionId:", currentSessionId);
      setStreaming(false);

      // The backend persists the assistant to the database before sending
      // message.complete, so the API response includes all messages.
      // We still build a local version from streamed segments as the
      // authoritative source for the current turn's display content.
      const round = streamingRef.current;

      if (currentSessionId && round) {
        const textContent = round.segments
          .filter((s) => s.type === "text")
          .map((s) => s.content)
          .join("");
        const reasoningContent = round.segments
          .filter((s) => s.type === "reasoning")
          .map((s) => s.content)
          .join("\n\n");

        // Collect tool calls that were fully streamed
        const toolCalls: ToolCall[] = Object.values(round.toolCallMap)
          .filter((e) => e.argumentsComplete)
          .map((e) => ({ id: e.id, name: e.name, arguments: e.arguments }));

        const assistantMsg: Message = {
          id: `stream-final-${Date.now()}`,
          role: "assistant",
          content: textContent || "",
          ...(reasoningContent ? { reasoning: reasoningContent } : {}),
          ...(toolCalls.length > 0 ? { tool_calls: toolCalls } : {}),
          created_at: new Date().toISOString(),
          completed_at: new Date().toISOString(),
        };

        console.log("[SSE] constructing assistant from stream, text:", textContent.length, "chars, toolCalls:", toolCalls.length);

        // Fetch messages from the API (backend now persists the assistant
        // before sending message.complete, so apiMessages includes all
        // user + assistant messages).  Replace the last assistant with our
        // locally-constructed version to ensure the displayed content matches
        // exactly what was streamed.
        api.listMessages(currentSessionId).then(({ messages: apiMessages }) => {
          const msgs = [...apiMessages];
          const lastIdx = msgs.length - 1;
          if (lastIdx >= 0 && msgs[lastIdx].role === "assistant") {
            msgs[lastIdx] = assistantMsg;
          } else {
            msgs.push(assistantMsg);
          }
          console.log("[SSE] merging API msgs (", apiMessages.length, ") with constructed assistant, total:", msgs.length);
          setMessages(msgs);
          useSessionStore.getState().setCurrentUsageStats(null);
        });
      } else {
        useSessionStore.getState().setCurrentUsageStats(null);
      }

      streamingRef.current = null;
      setStreamingRound(null);
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

    const handlePermissionRequest = (event: AppEvent) => {
      usePermissionStore.getState().handlePermissionRequestEvent(event);
    };

    // Register SSE listeners
    sseClient.on("tool.call", handleToolCall);
    sseClient.on("tool.result", handleToolResult);
    sseClient.on("shell.output", handleShellOutput);
    sseClient.on("message.chunk", handleMessageChunk);
    sseClient.on("reasoning.chunk", handleReasoningChunk);
    sseClient.on("message.complete", handleMessageComplete);
    sseClient.on("usage.stats", handleUsageStats);
    sseClient.on("error", handleErrorEvent);
    sseClient.on("aborted", handleAborted);
    sseClient.on("connected", handleConnected);
    sseClient.on("messages.updated", handleMessagesUpdated);
    sseClient.on("permission.request", handlePermissionRequest);

    // Connect
    sseClient.connect(sessionId);

    return () => {
      sseClient.off("tool.call", handleToolCall);
      sseClient.off("tool.result", handleToolResult);
      sseClient.off("shell.output", handleShellOutput);
      sseClient.off("message.chunk", handleMessageChunk);
      sseClient.off("reasoning.chunk", handleReasoningChunk);
      sseClient.off("message.complete", handleMessageComplete);
      sseClient.off("usage.stats", handleUsageStats);
      sseClient.off("error", handleErrorEvent);
      sseClient.off("aborted", handleAborted);
      sseClient.off("connected", handleConnected);
      sseClient.off("messages.updated", handleMessagesUpdated);
      sseClient.off("permission.request", handlePermissionRequest);
      sseClient.disconnect();
      setStreamingRound(null);
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
