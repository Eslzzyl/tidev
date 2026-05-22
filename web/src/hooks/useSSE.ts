import { useEffect, useRef } from "react";
import { sseClient } from "../api/sse";
import { usePermissionStore } from "../stores/usePermissionStore";
import { useSessionStore } from "../stores/useSessionStore";
import { useUIStore } from "../stores/useUIStore";
import { api } from "../api/client";
import type { AppEvent } from "../types/events";
import type { UsageStatsData } from "../stores/useSessionStore";
import type { Message, ToolCall } from "../types/api";

export function useSSE(sessionId: string | null) {
  const setMessages = useSessionStore((s) => s.setMessages);
  const setStreaming = useUIStore((s) => s.setStreaming);
  const setConnectionStatus = useUIStore((s) => s.setConnectionStatus);
  const currentSessionId = useSessionStore((s) => s.currentSessionId);

  // Tracks the request_id of the current LLM turn.
  // When a new turn starts (new request_id), we create a fresh streaming assistant message.
  const currentRequestIdRef = useRef<number | null>(null);

  // Tracks the message ID of the currently-streaming assistant message.
  // Used by handlers to quickly locate the message to update.
  const streamingAssistantIdRef = useRef<string | null>(null);

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  /**
   * Ensure a streaming assistant message exists for the given request_id.
   * If request_id matches the current turn, the existing message is reused.
   * If it's a new turn, a fresh assistant message is appended to the messages store.
   */
  function ensureStreamingAssistant(request_id: number): void {
    if (
      request_id === currentRequestIdRef.current &&
      streamingAssistantIdRef.current
    ) {
      return; // Same turn, assistant already exists
    }

    // New turn — create a fresh streaming assistant message
    currentRequestIdRef.current = request_id;

    // Ensure streaming is active — message_complete may have cleared it
    // after the previous turn, but the agent loop continues with more turns.
    setStreaming(true);

    const state = useSessionStore.getState();
    const msgs = [...state.messages];

    const newMsg: Message = {
      id: `stream-asst-${request_id}-${Date.now()}`,
      role: "assistant",
      content: "",
      created_at: new Date().toISOString(),
      reasoning: undefined,
      tool_calls: undefined,
      streaming: true,
    };

    streamingAssistantIdRef.current = newMsg.id;
    msgs.push(newMsg);
    state.setMessages(msgs);

    console.log(
      "[SSE] new streaming assistant (request_id=%s): %s",
      request_id,
      newMsg.id.substring(0, 20),
    );
  }

  /**
   * Update the streaming assistant message in the store by applying `updater`
   * to the current message (identified by streamingAssistantIdRef).
   * Silently no-ops if the streaming assistant is not found (race with abort/error).
   */
  function updateStreamingAssistant(updater: (msg: Message) => Message): void {
    const id = streamingAssistantIdRef.current;
    if (!id) return;

    const state = useSessionStore.getState();
    const msgs = state.messages.map((m) => (m.id === id ? updater(m) : m));
    state.setMessages(msgs);
  }

  // ---------------------------------------------------------------------------
  // Event handlers (defined inside useEffect so they see the latest closure)
  // ---------------------------------------------------------------------------

  useEffect(() => {
    if (!sessionId) return;

    setConnectionStatus("connecting");

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

    const handleMessageChunk = (event: AppEvent) => {
      if (event.type !== "message_chunk") return;
      ensureStreamingAssistant(event.request_id);
      updateStreamingAssistant((msg) => ({
        ...msg,
        content: msg.content + event.content,
      }));
    };

    const handleReasoningChunk = (event: AppEvent) => {
      if (event.type !== "reasoning_chunk") return;
      ensureStreamingAssistant(event.request_id);
      updateStreamingAssistant((msg) => ({
        ...msg,
        reasoning: (msg.reasoning ?? "") + event.content,
      }));
    };

    const handleToolCall = (event: AppEvent) => {
      if (event.type !== "tool_call") return;
      ensureStreamingAssistant(event.request_id);

      updateStreamingAssistant((msg) => {
        const toolCalls = [...(msg.tool_calls ?? [])];
        const existingIdx = toolCalls.findIndex(
          (tc) => tc.id === event.tool_call_id,
        );

        if (existingIdx >= 0) {
          // Update arguments of an existing tool call (streaming args)
          toolCalls[existingIdx] = {
            ...toolCalls[existingIdx],
            arguments: event.arguments || toolCalls[existingIdx].arguments,
          };
        } else {
          // New tool call
          toolCalls.push({
            id: event.tool_call_id,
            name: event.tool_name,
            arguments: event.arguments ?? "",
          });
        }

        return { ...msg, tool_calls: toolCalls };
      });
    };

    const handleToolResult = (event: AppEvent) => {
      if (event.type !== "tool_result") return;

      const state = useSessionStore.getState();
      const msgs = [...state.messages];

      // Append a tool result message so buildRounds can link it to the tool call
      const toolMsg: Message = {
        id: `tool-${event.tool_call_id}-${Date.now()}`,
        role: "tool",
        content: event.output,
        tool_call_id: event.tool_call_id,
        tool_name: undefined,
        created_at: new Date().toISOString(),
        diff: event.diff,
        filepath: event.filepath,
        rtk_rewritten: event.rtk_rewritten ?? false,
      };
      msgs.push(toolMsg);
      state.setMessages(msgs);

      console.log(
        "[SSE] tool result for %s (output: %d chars)",
        event.tool_call_id.substring(0, 12),
        event.output.length,
      );
    };

    const handleShellOutput = (event: AppEvent) => {
      if (event.type !== "shell_output") return;
      const { content, finished, exit_code } = event;

      updateStreamingAssistant((msg) => {
        const toolCalls = msg.tool_calls ? [...msg.tool_calls] : [];
        // Find the most recently added bash tool call
        let targetIdx = -1;
        for (let i = toolCalls.length - 1; i >= 0; i--) {
          if (toolCalls[i].name === "bash") {
            targetIdx = i;
            break;
          }
        }
        if (targetIdx < 0) return msg;

        // Parse exit code from content if present
        let exitCode: number | null = exit_code ?? null;
        let cleanContent = content;
        const exitMatch = content.match(/^\[exit\s*(-?\d+)\]\n/);
        if (exitMatch) {
          exitCode = parseInt(exitMatch[1], 10);
          cleanContent = content.slice(exitMatch[0].length);
        }

        const isError = exitCode !== null && exitCode !== 0;

        // Store shell output inline in the tool call's arguments (as JSON)
        // so ToolCallRow can display it.
        // We keep the original command and add output fields.
        let argsObj: Record<string, unknown> = {};
        try {
          argsObj = JSON.parse(toolCalls[targetIdx].arguments || "{}");
        } catch {
          // Not valid JSON yet; keep as-is
        }

        if (finished) {
          argsObj._output = cleanContent;
          argsObj._exitCode = exitCode;
          argsObj._isError = isError;
        } else {
          argsObj._partialOutput = cleanContent;
        }

        toolCalls[targetIdx] = {
          ...toolCalls[targetIdx],
          arguments: JSON.stringify(argsObj),
        };

        return { ...msg, tool_calls: toolCalls };
      });
    };

    const handleMessageComplete = () => {
      // Mark the streaming assistant as complete.
      // Keep isStreaming = true only if there are tool_calls → more turns follow.
      // If no tool_calls, this is the final turn → end streaming.
      if (streamingAssistantIdRef.current) {
        // Check if the completed assistant requested tools (more turns coming)
        const state = useSessionStore.getState();
        const completedMsg = state.messages.find(
          (m) => m.id === streamingAssistantIdRef.current,
        );
        const hasToolCalls =
          completedMsg?.tool_calls && completedMsg.tool_calls.length > 0;

        if (!hasToolCalls) {
          setStreaming(false);
        }

        updateStreamingAssistant((msg) => ({
          ...msg,
          completed_at: new Date().toISOString(),
          streaming: false,
        }));
        streamingAssistantIdRef.current = null;
      } else {
        // No streaming assistant — clean up streaming state
        setStreaming(false);
      }

      useSessionStore.getState().setCurrentUsageStats(null);
    };

    const handleErrorEvent = (event: AppEvent) => {
      if (!event || event.type !== "error") return;
      setStreaming(false);

      if (streamingAssistantIdRef.current) {
        updateStreamingAssistant((msg) => ({
          ...msg,
          content: msg.content
            ? `${msg.content}\n\n**Error**: ${event.message || "An error occurred"}`
            : `**Error**: ${event.message || "An error occurred"}`,
          completed_at: new Date().toISOString(),
          streaming: false,
        }));
        streamingAssistantIdRef.current = null;
      }
    };

    const handleRetrying = (event: AppEvent) => {
      if (event.type !== "retrying") return;
      // Show retry info on the UI store level
    };

    const handleAborted = () => {
      setStreaming(false);

      if (streamingAssistantIdRef.current) {
        // Mark the streaming assistant as complete (keep what was generated)
        updateStreamingAssistant((msg) => ({
          ...msg,
          completed_at: new Date().toISOString(),
          streaming: false,
        }));
        streamingAssistantIdRef.current = null;
      }
    };

    const handleConnected = () => {
      setConnectionStatus("connected");
    };

    const handleDisconnected = () => {
      setConnectionStatus("disconnected");
      setStreaming(false);
    };

    const handleMessagesUpdated = () => {
      // Refresh messages from API (e.g. after compaction completes).
      // Ignore during streaming to avoid wiping local streaming state.
      if (streamingAssistantIdRef.current) return;

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
    sseClient.on("message.chunk", handleMessageChunk);
    sseClient.on("reasoning.chunk", handleReasoningChunk);
    sseClient.on("tool.call", handleToolCall);
    sseClient.on("tool.result", handleToolResult);
    sseClient.on("shell.output", handleShellOutput);
    sseClient.on("message.complete", handleMessageComplete);
    sseClient.on("usage.stats", handleUsageStats);
    sseClient.on("error", handleErrorEvent);
    sseClient.on("retrying", handleRetrying);
    sseClient.on("aborted", handleAborted);
    sseClient.on("connected", handleConnected);
    sseClient.on("messages.updated", handleMessagesUpdated);
    sseClient.on("permission.request", handlePermissionRequest);

    // Connect
    sseClient.connect(sessionId);

    // Cleanup
    return () => {
      sseClient.off("message.chunk", handleMessageChunk);
      sseClient.off("reasoning.chunk", handleReasoningChunk);
      sseClient.off("tool.call", handleToolCall);
      sseClient.off("tool.result", handleToolResult);
      sseClient.off("shell.output", handleShellOutput);
      sseClient.off("message.complete", handleMessageComplete);
      sseClient.off("usage.stats", handleUsageStats);
      sseClient.off("error", handleErrorEvent);
      sseClient.off("retrying", handleRetrying);
      sseClient.off("aborted", handleAborted);
      sseClient.off("connected", handleConnected);
      sseClient.off("messages.updated", handleMessagesUpdated);
      sseClient.off("permission.request", handlePermissionRequest);
      sseClient.disconnect();
      setStreaming(false);
      streamingAssistantIdRef.current = null;
      currentRequestIdRef.current = null;
    };
  }, [
    sessionId,
    currentSessionId,
    setMessages,
    setStreaming,
    setConnectionStatus,
  ]);
}
