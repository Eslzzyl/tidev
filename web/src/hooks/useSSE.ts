import { useEffect, useRef } from "react";
import { sseClient } from "../api/sse";
import { usePermissionStore } from "../stores/usePermissionStore";
import { useSessionStore } from "../stores/useSessionStore";
import { useSubagentStore } from "../stores/useSubagentStore";
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

      // Try 1: bash tool path — find bash tool call in the streaming assistant
      let handled = false;
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
        handled = true;

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

      if (handled) return;

      // Try 2: ! shell mode path — find or create a shell output message
      const state = useSessionStore.getState();
      const msgs = [...state.messages];

      // Find the last shell message with streaming state (might have been optimistically created)
      let found = false;
      for (let i = msgs.length - 1; i >= 0; i--) {
        // Look for a shell message that has no content yet (streaming placeholder),
        // or the last shell message that's not the command ($ prefix)
        if (msgs[i].role === "shell" && msgs[i].streaming) {
          msgs[i] = {
            ...msgs[i],
            content,
            streaming: !finished,
            completed_at: finished ? new Date().toISOString() : undefined,
          };
          found = true;
          break;
        }
      }

      if (found) {
        state.setMessages(msgs);
        return;
      }

      // No existing streaming shell message — create a new one
      if (finished) {
        const shellOutput: Message = {
          id: `shell-out-${Date.now()}`,
          role: "shell",
          content,
          created_at: new Date().toISOString(),
          completed_at: new Date().toISOString(),
        };
        state.setMessages([...msgs, shellOutput]);
      }
    };

    const handleMessageComplete = () => {
      // Mark the streaming assistant as complete.
      // Do NOT set streaming=false here — the stream.end event (sent by the
      // agent loop when it fully exits) is the authoritative signal.  This
      // prevents the footer from flashing during multi-turn agent loops
      // where tool.calls arrive after message.complete.
      if (streamingAssistantIdRef.current) {
        const stats = useSessionStore.getState().currentUsageStats;
        updateStreamingAssistant((msg) => ({
          ...msg,
          completed_at: new Date().toISOString(),
          streaming: false,
          // Persist usage stats onto the message so the sidebar can read them
          // even after currentUsageStats is cleared by stream.end.
          token_usage: stats
            ? {
                total_tokens: stats.total_tokens,
                input_tokens: stats.input_tokens,
                output_tokens: stats.output_tokens,
                cache_read_tokens: stats.cache_read_tokens,
                cache_write_tokens: stats.cache_write_tokens,
              }
            : msg.token_usage,
          tokens_per_second:
            stats?.tokens_per_second ?? msg.tokens_per_second,
        }));
        streamingAssistantIdRef.current = null;
      } else {
        setStreaming(false);
      }
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

    const handleStreamEnd = () => {
      setStreaming(false);
      useSessionStore.getState().setCurrentUsageStats(null);
      streamingAssistantIdRef.current = null;
      currentRequestIdRef.current = null;
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

    // -----------------------------------------------------------------------
    // Subagent event handlers
    // -----------------------------------------------------------------------

    /**
     * Find a task tool call that hasn't been mapped to a child session yet.
     * Searches all assistant messages (including the streaming one) to handle
     * subagent events that arrive after the parent's message has been finalized.
     */
    function findUnmappedTaskToolCall(): ToolCall | null {
      const state = useSessionStore.getState();
      const subagentStore = useSubagentStore.getState();

      // Search all assistant messages for task tool calls
      for (const msg of state.messages) {
        if (msg.role !== "assistant") continue;
        if (!msg.tool_calls || msg.tool_calls.length === 0) continue;
        const match = msg.tool_calls.find(
          (tc) =>
            tc.name === "task" &&
            !subagentStore.states[tc.id],
        );
        if (match) return match;
      }
      return null;
    }

    const handleSubagentStatus = (event: AppEvent) => {
      if (event.type !== "subagent_status") return;
      const {
        child_session_id,
        status_text,
        content_delta,
        reasoning_delta,
        current_tool_name,
        current_tool_args,
      } = event;

      // Try to find existing mapping by child_session_id
      const subagentStore = useSubagentStore.getState();
      let toolCallId: string | null = null;

      for (const [tcId, state] of Object.entries(subagentStore.states)) {
        if (state.childSessionId === child_session_id) {
          toolCallId = tcId;
          break;
        }
      }

      // If no existing mapping, find a task tool call not yet mapped
      if (!toolCallId) {
        const work = findUnmappedTaskToolCall();
        if (work) {
          toolCallId = work.id;
        }
      }

      if (!toolCallId) return;

      const current = subagentStore.states[toolCallId] || {
        completed: false,
        blocks: [],
      };
      const blocks = [...current.blocks];

      // --- Handle reasoning delta ---
      if (reasoning_delta) {
        const last = blocks[blocks.length - 1];
        if (last?.type === "reasoning") {
          // Append to existing reasoning block
          last.content = (last.content ?? "") + reasoning_delta;
        } else {
          // Start a new reasoning block
          blocks.push({ type: "reasoning", content: reasoning_delta });
        }
      }

      // --- Handle content delta ---
      if (content_delta) {
        const last = blocks[blocks.length - 1];
        if (last?.type === "content") {
          // Append to existing content block
          last.content = (last.content ?? "") + content_delta;
        } else {
          // Start a new content block
          blocks.push({ type: "content", content: content_delta });
        }
      }

      // --- Handle tool call ---
      if (current_tool_name) {
        // Check if the last block is the same tool (still running)
        const last = blocks[blocks.length - 1];
        if (
          last?.type === "tool_call" &&
          last.toolName === current_tool_name &&
          !last.complete
        ) {
          // Update existing tool block with fresher args
          last.toolArgs = current_tool_args;
        } else {
          // Mark previous tool call as complete if it was a tool block
          if (last?.type === "tool_call" && !last.complete) {
            last.complete = true;
          }
          // Push new tool call block
          blocks.push({
            type: "tool_call",
            toolName: current_tool_name,
            toolArgs: current_tool_args,
            complete: false,
          });
        }
      } else if (
        // When status is Working/Completed (no tool running), mark last tool as done
        !current_tool_name &&
        (status_text === "Working" || status_text === "Completed")
      ) {
        const last = blocks[blocks.length - 1];
        if (last?.type === "tool_call" && !last.complete) {
          last.complete = true;
        }
      }

      // Update the store
      useSubagentStore.getState().updateState(toolCallId, {
        childSessionId: child_session_id,
        statusText: status_text,
        blocks,
      });
    };

    const handleSubagentToolResult = (event: AppEvent) => {
      if (event.type !== "subagent_tool_result") return;
      const { child_session_id, content_delta, reasoning_delta } = event;
      if (!content_delta && !reasoning_delta) return;

      // Find the tool call by child session id and append to blocks
      const subagentStore = useSubagentStore.getState();
      for (const [tcId, state] of Object.entries(subagentStore.states)) {
        if (state.childSessionId === child_session_id) {
          const blocks = [...(state.blocks || [])];

          if (reasoning_delta) {
            const last = blocks[blocks.length - 1];
            if (last?.type === "reasoning") {
              last.content = (last.content ?? "") + reasoning_delta;
            } else {
              blocks.push({ type: "reasoning", content: reasoning_delta });
            }
          }

          if (content_delta) {
            const last = blocks[blocks.length - 1];
            if (last?.type === "content") {
              last.content = (last.content ?? "") + content_delta;
            } else {
              blocks.push({ type: "content", content: content_delta });
            }
          }

          useSubagentStore.getState().updateState(tcId, { blocks });
          break;
        }
      }
    };

    const handleSubagentCompleted = (event: AppEvent) => {
      if (event.type !== "subagent_completed") return;
      const { tool_call_id, child_session_id } = event;

      // Update subagent store — the result is already handled by
      // the ToolCompleted event → handleToolResult, so we just mark
      // the subagent as complete and store the child session ID.
      useSubagentStore.getState().updateState(tool_call_id, {
        childSessionId: child_session_id,
        completed: true,
      });

      console.log(
        "[SSE] subagent completed for %s",
        tool_call_id.substring(0, 12),
      );
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
    sseClient.on("subagent.status", handleSubagentStatus);
    sseClient.on("subagent.tool_result", handleSubagentToolResult);
    sseClient.on("subagent.completed", handleSubagentCompleted);
    sseClient.on("stream.end", handleStreamEnd);

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
      sseClient.off("subagent.status", handleSubagentStatus);
      sseClient.off("subagent.tool_result", handleSubagentToolResult);
      sseClient.off("subagent.completed", handleSubagentCompleted);
      sseClient.off("stream.end", handleStreamEnd);
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
