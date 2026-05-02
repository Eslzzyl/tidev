import { useEffect, useRef, useState, useCallback } from 'react';
import { sseClient } from '../api/sse';
import { useSessionStore } from '../stores/useSessionStore';
import { useUIStore } from '../stores/useUIStore';
import { api } from '../api/client';
import type { AppEvent } from '../types/events';
import type { Round, ToolCallEntry } from '../types/round';

function checkStartStreamingRound(
  completedRounds: Round[],
  currentStreaming: Round | null
): Round | null {
  if (currentStreaming) return currentStreaming;
  for (let i = completedRounds.length - 1; i >= 0; i--) {
    const r = completedRounds[i];
    if (r.status === 'user_only' && r.segments.length === 0) {
      return {
        id: `streaming-${r.userMessage.id}`,
        userMessage: r.userMessage,
        segments: [],
        toolCallMap: {},
        status: 'streaming' as const,
      };
    }
  }
  return null;
}

export function useSSE(sessionId: string | null) {
  const [streamingRound, setStreamingRound] = useState<Round | null>(null);
  const streamingRef = useRef<Round | null>(null);

  const setMessages = useSessionStore((s) => s.setMessages);
  const setStreaming = useUIStore((s) => s.setStreaming);
  const setConnectionStatus = useUIStore((s) => s.setConnectionStatus);
  const currentSessionId = useSessionStore((s) => s.currentSessionId);

  const updateStreamingRound = useCallback((updater: (prev: Round | null) => Round | null) => {
    setStreamingRound((prev) => {
      const next = updater(prev);
      streamingRef.current = next;
      return next;
    });
  }, []);

  useEffect(() => {
    if (!sessionId) return;

    setConnectionStatus('connecting');

    const handleToolCall = (event: AppEvent) => {
      if (event.type !== 'tool_call') return;
      const current = streamingRef.current;
      if (!current) return;

      const { tool_call_id, tool_name, arguments: args } = event;
      const toolCallMap = { ...current.toolCallMap };
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
          JSON.parse(args || '');
          argsComplete = true;
        } catch {
          // Still streaming
        }

        toolCallMap[tool_call_id] = {
          id: tool_call_id,
          name: tool_name,
          arguments: args || '',
          argumentsComplete: argsComplete,
          resultComplete: false,
        };
      }

      updateStreamingRound((prev) => {
        if (!prev) return prev;
        return { ...prev, toolCallMap: { ...toolCallMap } };
      });
    };

    const handleToolResult = (event: AppEvent) => {
      if (event.type !== 'tool_result') return;
      const current = streamingRef.current;
      if (!current) return;

      const { tool_call_id, output, diff, filepath, rtk_rewritten } = event;
      const toolCallMap = { ...current.toolCallMap };
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

      updateStreamingRound((prev) => {
        if (!prev) return prev;
        return { ...prev, toolCallMap: { ...toolCallMap } };
      });
    };

    const handleMessageChunk = (event: AppEvent) => {
      if (event.type !== 'message_chunk') return;
      const current = streamingRef.current;

      if (!current) {
        // Try to start a streaming round
        const completedRounds = []; // We can't easily compute this here
        // Instead, create a minimal streaming round
        updateStreamingRound((prev) => {
          if (prev) {
            const segments = [...prev.segments];
            const lastSeg = segments[segments.length - 1];
            if (lastSeg && lastSeg.type === 'text') {
              lastSeg.content += event.content;
            } else {
              segments.push({ type: 'text', content: event.content });
            }
            return { ...prev, segments };
          }
          return prev;
        });
        return;
      }

      const segments = [...current.segments];
      const lastSeg = segments[segments.length - 1];
      if (lastSeg && lastSeg.type === 'text') {
        lastSeg.content += event.content;
      } else {
        segments.push({ type: 'text', content: event.content });
      }

      updateStreamingRound((prev) => {
        if (!prev) return prev;
        return { ...prev, segments };
      });
    };

    const handleMessageComplete = () => {
      setStreaming(false);

      // Finalize the streaming round
      updateStreamingRound((prev) => {
        if (!prev) return prev;
        return {
          ...prev,
          status: 'complete',
          completedAt: new Date().toISOString(),
        };
      });

      // Refresh messages from API
      if (currentSessionId) {
        api.listMessages(currentSessionId).then(({ messages }) => {
          setMessages(messages);
          streamingRef.current = null;
          setStreamingRound(null);
        });
      } else {
        streamingRef.current = null;
        setStreamingRound(null);
      }
    };

    const handleErrorEvent = (event: AppEvent) => {
      if (event.type === 'error') {
        setStreaming(false);
        setStreamingRound(null);
        streamingRef.current = null;
      } else {
        setConnectionStatus('disconnected');
      }
    };

    const handleAborted = () => {
      setStreaming(false);
      setStreamingRound(null);
      streamingRef.current = null;
    };

    const handleConnected = () => {
      setConnectionStatus('connected');
    };

    // Register SSE listeners
    sseClient.on('tool.call', handleToolCall);
    sseClient.on('tool.result', handleToolResult);
    sseClient.on('message.chunk', handleMessageChunk);
    sseClient.on('message.complete', handleMessageComplete);
    sseClient.on('error', handleErrorEvent);
    sseClient.on('aborted', handleAborted);
    sseClient.on('connected', handleConnected);

    // Connect
    sseClient.connect(sessionId);

    return () => {
      sseClient.off('tool.call', handleToolCall);
      sseClient.off('tool.result', handleToolResult);
      sseClient.off('message.chunk', handleMessageChunk);
      sseClient.off('message.complete', handleMessageComplete);
      sseClient.off('error', handleErrorEvent);
      sseClient.off('aborted', handleAborted);
      sseClient.off('connected', handleConnected);
      sseClient.disconnect();
      streamingRef.current = null;
    };
  }, [sessionId, currentSessionId, setMessages, setStreaming, setConnectionStatus, updateStreamingRound]);

  return streamingRound;
}
