import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api } from "../api/client";
import { openBackendEvents, openFrontendRequests } from "../api/events";
import { useAuthStore } from "../stores/useAuthStore";
import { useUIStore } from "../stores/useUIStore";
import type {
  ApprovedTool,
  EventEnvelope,
  FrontendRequest,
  Message,
  MessageRecord,
  Model,
  ProviderErrorData,
  Session,
  SessionListCursor,
  TodoItem,
  ToolCall,
  ToolExecutionResult,
} from "../types/api";
import type { InstructionNotice, StreamMessage } from "../types/chat";
import { parseSlashCommand } from "../commands";
import { asString, eventPayload } from "../utils/events";
import { toolCallEntry, toolResultStatus, type ToolCallEntry } from "../utils/round";
import i18n from "../i18n";

const SESSION_PAGE_SIZE = 50;

function mergeSessions(current: Session[], incoming: Session[]): Session[] {
  const seen = new Set<string>();
  return [...current, ...incoming].filter((session) => {
    if (seen.has(session.session_id)) return false;
    seen.add(session.session_id);
    return true;
  });
}

function createStream(key: string, requestId: number): StreamMessage {
  return {
    key,
    requestId,
    segments: [],
    toolCallMap: {},
    status: "streaming",
    providerFinished: false,
    reasoningStartedAt: null,
    reasoningCompletedAt: null,
  };
}

function cloneStream(stream: StreamMessage): StreamMessage {
  return {
    ...stream,
    segments: stream.segments.slice(),
    toolCallMap: { ...stream.toolCallMap },
  };
}

function freezeReasoning(stream: StreamMessage) {
  const lastIndex = stream.segments.length - 1;
  const last = stream.segments[lastIndex];
  if (last?.type !== "reasoning" || !stream.reasoningStartedAt) return;

  const completedAt = stream.reasoningCompletedAt ?? new Date().toISOString();
  stream.reasoningCompletedAt = completedAt;
  if (!last.completedAt) {
    stream.segments[lastIndex] = { ...last, completedAt };
  }
}

function appendSegment(stream: StreamMessage, type: "text" | "reasoning", content: string) {
  if (!content) return;
  if (type === "text") freezeReasoning(stream);
  const lastIndex = stream.segments.length - 1;
  const last = stream.segments[lastIndex];
  if (last?.type === type) {
    stream.segments[lastIndex] = { ...last, content: last.content + content };
  } else {
    stream.segments.push({ type, content });
  }
}

function reconcileSegment(stream: StreamMessage, type: "text" | "reasoning", content: string) {
  if (!content) return;
  const index = stream.segments.findLastIndex((segment) => segment.type === type);
  const existing = index >= 0 ? stream.segments[index] : undefined;
  if (!existing || existing.type !== type) {
    stream.segments.push({ type, content });
    return;
  }
  if (existing.content === content || existing.content.endsWith(content)) return;
  if (content.startsWith(existing.content)) {
    stream.segments[index] = { ...existing, content };
  } else if (!existing.content.includes(content)) {
    stream.segments.push({ type, content });
  }
}

function ensureToolCall(stream: StreamMessage, toolCall: ToolCall): ToolCallEntry {
  freezeReasoning(stream);
  const existing = stream.toolCallMap[toolCall.id];
  if (existing) {
    const updated = { ...existing, name: toolCall.name, arguments: toolCall.arguments };
    stream.toolCallMap[toolCall.id] = updated;
    return updated;
  }

  const entry = toolCallEntry(toolCall);
  stream.toolCallMap[toolCall.id] = entry;
  stream.segments.push({ type: "tool_call", toolCallId: toolCall.id });
  return entry;
}

function emptyToolMetadata() {
  return {
    filepath: null,
    diff: null,
    truncated: null,
    exists: null,
    prior_summary: null,
    prior_retained_from: null,
    file_changes: [],
    exit_code: null,
    duration_ms: null,
  };
}

function updateShellResult(
  entry: ToolCallEntry,
  content: string,
  finished: boolean,
  exitCode: number | null,
): void {
  const result: ToolExecutionResult = {
    output: content,
    attachments: entry.result?.attachments ?? [],
    metadata: {
      ...(entry.result?.metadata ?? emptyToolMetadata()),
      exit_code: exitCode ?? entry.result?.metadata.exit_code ?? null,
    },
  };
  entry.result = result;
  entry.status = finished ? toolResultStatus(result) : "running";
}

function toolCallFromPayload(value: unknown): ToolCall | null {
  if (!value || typeof value !== "object") return null;
  const candidate = value as Partial<ToolCall>;
  if (
    typeof candidate.id !== "string" ||
    typeof candidate.name !== "string" ||
    typeof candidate.arguments !== "string"
  ) {
    return null;
  }
  return {
    id: candidate.id,
    name: candidate.name,
    arguments: candidate.arguments,
    thought_signature: candidate.thought_signature ?? null,
  };
}

function toolResultFromPayload(value: unknown): ToolExecutionResult | null {
  if (!value || typeof value !== "object") return null;
  const candidate = value as Partial<ToolExecutionResult>;
  if (
    typeof candidate.output !== "string" ||
    !Array.isArray(candidate.attachments) ||
    !candidate.metadata ||
    typeof candidate.metadata !== "object"
  ) {
    return null;
  }
  return candidate as ToolExecutionResult;
}

function providerErrorFromPayload(
  value: unknown,
  retryableValue: unknown,
  requestId: number,
  userMessageId: string | null,
): ProviderErrorData {
  const candidate = value && typeof value === "object" ? (value as Record<string, unknown>) : null;
  const message = candidate ? asString(candidate.message) : asString(value);
  return {
    message: message || i18n.t("Response failed"),
    retryable:
      candidate && typeof candidate.retryable === "boolean"
        ? candidate.retryable
        : retryableValue === true,
    request_id:
      candidate && typeof candidate.request_id === "number" ? candidate.request_id : requestId,
    user_message_id:
      candidate && typeof candidate.user_message_id === "string"
        ? candidate.user_message_id
        : userMessageId,
  };
}

function updateSubagentEntry(
  stream: StreamMessage,
  toolCallId: string,
  childSessionId: string,
  statusText: string,
  currentToolCall: ToolCall | null,
  contentDelta: string,
  reasoningDelta: string,
): void {
  const entry = stream.toolCallMap[toolCallId];
  if (!entry) return;
  const updated: ToolCallEntry = {
    ...entry,
    status: entry.status === "completed" || entry.status === "failed" ? entry.status : "running",
    childSessionId,
    subagentStatus: statusText || entry.subagentStatus,
    subagentContentDelta: `${entry.subagentContentDelta ?? ""}${contentDelta}`,
    subagentReasoningDelta: `${entry.subagentReasoningDelta ?? ""}${reasoningDelta}`,
  };
  if (currentToolCall) {
    updated.name = currentToolCall.name;
  }
  stream.toolCallMap[toolCallId] = updated;
}

export interface UseChatRuntimeOptions {
  routeSessionId?: string | null;
  onSelectSessionRoute?: (sessionId: string | null) => void;
}

export function useChatRuntime(options?: UseChatRuntimeOptions) {
  const { routeSessionId, onSelectSessionRoute } = options ?? {};
  const authChecking = useAuthStore((state) => state.isLoading);
  const authRequired = useAuthStore((state) => state.isAuthRequired);
  const authenticated = useAuthStore((state) => state.isAuthenticated);
  const checkAuthStatus = useAuthStore((state) => state.checkAuthStatus);
  const openSettingsPanel = useUIStore((state) => state.openSettingsPanel);
  const enterToSend = useUIStore((state) => state.settings.enterToSend);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [selectedSession, setSelectedSession] = useState<Session | undefined>();
  const [sessionWorkspaceRoots, setSessionWorkspaceRoots] = useState<string[]>([]);
  const [sessionWorkspaceRoot, setSessionWorkspaceRoot] = useState<string | null>(null);
  const [nextSessionCursor, setNextSessionCursor] = useState<SessionListCursor | null>(null);
  const [loadingMoreSessions, setLoadingMoreSessions] = useState(false);
  const [messages, setMessages] = useState<MessageRecord[]>([]);
  const [instructionNotices, setInstructionNotices] = useState<InstructionNotice[]>([]);
  const [streams, setStreams] = useState<Record<string, StreamMessage>>({});
  const [requests, setRequests] = useState<FrontendRequest[]>([]);
  const [models, setModels] = useState<Model[]>([]);
  const [todos, setTodos] = useState<TodoItem[]>([]);
  const [draft, setDraft] = useState("");
  const pendingDraft = useUIStore((s) => s.pendingDraft);
  const setPendingDraft = useUIStore((s) => s.setPendingDraft);

  useEffect(() => {
    if (pendingDraft !== null) {
      setDraft(pendingDraft);
      setPendingDraft(null);
    }
  }, [pendingDraft, setPendingDraft]);
  const [mode, setMode] = useState<"build" | "plan">("build");
  const [loading, setLoading] = useState(true);
  const [sending, setSending] = useState(false);
  const [canceling, setCanceling] = useState(false);
  const [welcomeSending, setWelcomeSending] = useState(false);
  const [focusComposerAfterWelcome, setFocusComposerAfterWelcome] = useState(false);
  const [scrollToBottomRequest, setScrollToBottomRequest] = useState(0);
  const [thinkingLevel, setThinkingLevel] = useState<string | undefined>();
  const [sessionSearch, setSessionSearch] = useState("");
  const [debouncedSessionSearch, setDebouncedSessionSearch] = useState("");
  const [renamingSessionId, setRenamingSessionId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [fileMention, setFileMention] = useState<{ query: string; atPos: number } | null>(null);
  const [fileMentionIndex, setFileMentionIndex] = useState(0);
  const selectedSessionRef = useRef<string | null>(null);
  const sessionsRef = useRef<Session[]>([]);
  const instructionNoticesRef = useRef<InstructionNotice[]>([]);
  const instructionNoticeRevisionRef = useRef(0);
  const shownInstructionSourcesRef = useRef<Set<string>>(new Set());
  const instructionToolSessionsRef = useRef<Set<string>>(new Set());
  const cursorRef = useRef<number | null>(
    Number(localStorage.getItem("tidev:last-event-cursor")) || null,
  );
  useEffect(() => {
    sessionsRef.current = sessions;
  }, [sessions]);
  useEffect(() => {
    const timer = window.setTimeout(() => {
      setDebouncedSessionSearch(sessionSearch);
    }, 200);
    return () => window.clearTimeout(timer);
  }, [sessionSearch]);
  type StreamState = Record<string, StreamMessage>;
  type StreamStateUpdater = (current: StreamState) => StreamState;
  const pendingStreamUpdatesRef = useRef<StreamStateUpdater[]>([]);
  const streamUpdateFrameRef = useRef<number | null>(null);
  const streamUpdateFrameKindRef = useRef<"animation" | "timeout" | null>(null);

  const flushStreamUpdates = useCallback(() => {
    streamUpdateFrameRef.current = null;
    streamUpdateFrameKindRef.current = null;
    const pending = pendingStreamUpdatesRef.current.splice(0);
    if (pending.length === 0) return;
    setStreams((current) => pending.reduce((next, update) => update(next), current));
  }, []);

  const scheduleStreamUpdate = useCallback(
    (update: StreamStateUpdater) => {
      pendingStreamUpdatesRef.current.push(update);
      if (streamUpdateFrameRef.current !== null) return;
      if (typeof window.requestAnimationFrame === "function") {
        streamUpdateFrameKindRef.current = "animation";
        streamUpdateFrameRef.current = window.requestAnimationFrame(flushStreamUpdates);
      } else {
        streamUpdateFrameKindRef.current = "timeout";
        streamUpdateFrameRef.current = window.setTimeout(flushStreamUpdates, 0);
      }
    },
    [flushStreamUpdates],
  );

  const clearPendingStreamUpdates = useCallback(() => {
    if (streamUpdateFrameRef.current !== null) {
      if (streamUpdateFrameKindRef.current === "animation") {
        window.cancelAnimationFrame(streamUpdateFrameRef.current);
      } else {
        window.clearTimeout(streamUpdateFrameRef.current);
      }
      streamUpdateFrameRef.current = null;
      streamUpdateFrameKindRef.current = null;
    }
    pendingStreamUpdatesRef.current = [];
  }, []);

  useEffect(() => clearPendingStreamUpdates, [clearPendingStreamUpdates]);

  useEffect(() => {
    void checkAuthStatus();
  }, [checkAuthStatus]);

  const findAtFragment = useCallback(
    (text: string, cursor: number): { atPos: number; query: string } | null => {
      const safeCursor = Math.min(cursor, text.length);
      const prefix = text.slice(0, safeCursor);
      const atIndex = prefix.lastIndexOf("@");
      if (atIndex === -1) return null;
      if (atIndex > 0) {
        const prev = prefix[atIndex - 1];
        if (prev && !/\s/.test(prev) && !["(", "[", "{", '"', "/", "\\"].includes(prev))
          return null;
      }
      const query = prefix.slice(atIndex + 1);
      if (query.length > 0 && /\s/.test(query)) return null;
      return { atPos: atIndex, query };
    },
    [],
  );

  const updateFileMention = useCallback(
    (text: string, cursor: number) => {
      const fragment = findAtFragment(text, cursor);
      if (fragment) {
        setFileMention(fragment);
        setFileMentionIndex(0);
      } else {
        setFileMention(null);
      }
    },
    [findAtFragment],
  );

  const handleFileSelect = useCallback(
    (path: string) => {
      if (!fileMention) return undefined;
      const before = draft.slice(0, fileMention.atPos);
      const after = draft.slice(fileMention.atPos + 1 + fileMention.query.length);
      const inserted = `${before}@${path} ${after}`;
      setDraft(inserted);
      setFileMention(null);
      return before.length + path.length + 2;
    },
    [draft, fileMention],
  );

  const clearPendingInstructionNotices = useCallback(() => {
    instructionNoticesRef.current = [];
    instructionNoticeRevisionRef.current += 1;
    setInstructionNotices([]);
  }, []);

  const resetInstructionState = useCallback(() => {
    clearPendingInstructionNotices();
    shownInstructionSourcesRef.current.clear();
    instructionToolSessionsRef.current.clear();
  }, [clearPendingInstructionNotices]);

  const loadMessages = useCallback(async (sessionId: string) => {
    if (!sessionId?.trim()) return false;
    try {
      const response = await api.listMessages(sessionId);
      if (selectedSessionRef.current === sessionId) {
        setMessages(response.messages);
      }
      return true;
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to load messages"));
      return false;
    }
  }, []);

  const loadTodos = useCallback(async (sessionId: string) => {
    if (!sessionId?.trim()) return;
    try {
      const response = await api.getTodos(sessionId);
      if (selectedSessionRef.current === sessionId) setTodos(response.todos);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to load to-do list"));
    }
  }, []);

  const selectSession = useCallback(
    (sessionId: string, session?: Session, triggerRoute = true) => {
      selectedSessionRef.current = sessionId;
      setSelectedSessionId(sessionId);
      const summary = session ?? sessionsRef.current.find((item) => item.session_id === sessionId);
      if (summary) {
        setSelectedSession(summary);
      } else {
        setSelectedSession(undefined);
        void api
          .getSession(sessionId)
          .then((loaded) => {
            if (selectedSessionRef.current === sessionId) setSelectedSession(loaded);
          })
          .catch((reason) => {
            if (selectedSessionRef.current === sessionId) {
              setError(reason instanceof Error ? reason.message : i18n.t("Failed to load session"));
            }
          });
      }
      if (triggerRoute) {
        onSelectSessionRoute?.(sessionId);
      }
      clearPendingStreamUpdates();
      resetInstructionState();
      setStreams({});
      setTodos([]);
      setError(null);
      setFileMention(null);
      void loadMessages(sessionId);
      void loadTodos(sessionId);
    },
    [
      clearPendingStreamUpdates,
      loadMessages,
      loadTodos,
      onSelectSessionRoute,
      resetInstructionState,
    ],
  );

  const createSession = useCallback(
    (triggerRoute = true) => {
      selectedSessionRef.current = null;
      setSelectedSessionId(null);
      setSelectedSession(undefined);
      if (triggerRoute) {
        onSelectSessionRoute?.(null);
      }
      setSessionSearch("");
      setSessionWorkspaceRoot(null);
      setMessages([]);
      clearPendingStreamUpdates();
      resetInstructionState();
      setStreams({});
      setDraft("");
      setFileMention(null);
      setError(null);
    },
    [clearPendingStreamUpdates, onSelectSessionRoute, resetInstructionState],
  );

  useEffect(() => {
    if (routeSessionId === undefined) return;
    if (routeSessionId) {
      if (selectedSessionRef.current !== routeSessionId) {
        selectSession(
          routeSessionId,
          sessionsRef.current.find((item) => item.session_id === routeSessionId),
          false,
        );
      }
    } else if (selectedSessionRef.current !== null) {
      createSession(false);
    }
  }, [routeSessionId, selectSession, createSession]);

  const clearComposerFocusRequest = useCallback(() => {
    setFocusComposerAfterWelcome(false);
  }, []);

  useEffect(() => {
    if (authChecking || (authRequired && !authenticated)) return;
    let disposed = false;
    setLoading(true);
    void api
      .listSessions({
        limit: SESSION_PAGE_SIZE,
        query: debouncedSessionSearch,
        workspaceRoot: sessionWorkspaceRoot,
      })
      .then((page) => {
        if (disposed) return;
        setSessions(page.items);
        setSessionWorkspaceRoots(page.workspace_roots);
        setNextSessionCursor(page.next_cursor);
        setSelectedSession((current) =>
          current
            ? (page.items.find((session) => session.session_id === current.session_id) ?? current)
            : current,
        );

        if (routeSessionId && selectedSessionRef.current === null) {
          selectSession(
            routeSessionId,
            page.items.find((session) => session.session_id === routeSessionId),
            false,
          );
        }
      })
      .catch((reason) =>
        setError(reason instanceof Error ? reason.message : i18n.t("Failed to load sessions")),
      )
      .finally(() => {
        if (!disposed) {
          setLoading(false);
        }
      });
    return () => {
      disposed = true;
    };
  }, [
    authChecking,
    authRequired,
    authenticated,
    debouncedSessionSearch,
    routeSessionId,
    selectSession,
    sessionWorkspaceRoot,
  ]);

  const loadMoreSessions = useCallback(async () => {
    if (!nextSessionCursor || loadingMoreSessions) return;
    setLoadingMoreSessions(true);
    try {
      const page = await api.listSessions({
        limit: SESSION_PAGE_SIZE,
        query: debouncedSessionSearch,
        workspaceRoot: sessionWorkspaceRoot,
        cursor: nextSessionCursor,
      });
      setSessions((current) => mergeSessions(current, page.items));
      setSessionWorkspaceRoots(page.workspace_roots);
      setNextSessionCursor(page.next_cursor);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to load sessions"));
    } finally {
      setLoadingMoreSessions(false);
    }
  }, [debouncedSessionSearch, loadingMoreSessions, nextSessionCursor, sessionWorkspaceRoot]);

  const touchSession = useCallback((sessionId: string, busy?: boolean) => {
    const updatedAt = new Date().toISOString();
    const update = (session: Session): Session => ({
      ...session,
      updated_at: updatedAt,
      ...(busy === undefined ? {} : { busy }),
    });
    setSessions((current) => {
      const session = current.find((item) => item.session_id === sessionId);
      if (!session) return current;
      return [update(session), ...current.filter((item) => item.session_id !== sessionId)];
    });
    setSelectedSession((current) =>
      current?.session_id === sessionId ? update(current) : current,
    );
  }, []);

  const refreshModels = useCallback(async () => {
    try {
      const available = await api.listModels();
      setModels(available);
      setThinkingLevel(
        (current) => current ?? available.find((model) => model.active)?.thinking_level,
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to load models"));
    }
  }, []);

  useEffect(() => {
    void refreshModels();
  }, [refreshModels]);

  const applyEvent = useCallback(
    (envelope: EventEnvelope) => {
      cursorRef.current = envelope.cursor;
      localStorage.setItem("tidev:last-event-cursor", String(envelope.cursor));
      const [kind, payload] = eventPayload(envelope);
      const sessionId = asString(payload.session_id) || envelope.session_id;
      if (kind === "UserMessageCreated") {
        const message = payload.message as Message | undefined;
        const appData = payload.app_data as MessageRecord["app_data"] | undefined;
        touchSession(sessionId);
        if (message && payload.queued !== true && selectedSessionRef.current === sessionId) {
          setMessages((current) =>
            current.some((item) => item.message.id === message.id)
              ? current
              : [...current, { message, app_data: appData ?? {} }],
          );
        }
        return;
      }
      if (kind === "InstructionsLoaded") {
        if (selectedSessionRef.current !== sessionId || !Array.isArray(payload.sources)) return;
        const newSources = new Set<string>();
        const sources = payload.sources.filter((source): source is string => {
          if (
            typeof source !== "string" ||
            source.trim().length === 0 ||
            shownInstructionSourcesRef.current.has(source) ||
            newSources.has(source)
          ) {
            return false;
          }
          newSources.add(source);
          return true;
        });
        if (sources.length === 0) return;
        for (const source of sources) shownInstructionSourcesRef.current.add(source);
        const merged = [
          ...instructionNoticesRef.current,
          { sources, deferred: instructionToolSessionsRef.current.has(sessionId) },
        ];
        instructionNoticesRef.current = merged;
        instructionNoticeRevisionRef.current += 1;
        setInstructionNotices(merged);
        return;
      }
      if (kind === "TurnStarting") {
        const requestId = Number(payload.request_id);
        if (Number.isFinite(requestId)) {
          const key = `${sessionId}:${requestId}`;
          scheduleStreamUpdate((current) => {
            const stream = current[key] ?? createStream(key, requestId);
            return {
              ...current,
              [key]: {
                ...stream,
                status: "streaming",
                providerFinished: false,
                error: undefined,
                providerError: undefined,
                retrying: undefined,
                userMessageId: asString(payload.user_message_id) || stream.userMessageId || null,
              },
            };
          });
        }
        touchSession(sessionId, true);
        return;
      }
      if (
        kind === "Delta" ||
        kind === "ReasoningDelta" ||
        kind === "ReasoningSummaryDelta" ||
        kind === "ToolCallUpdated"
      ) {
        const requestId = Number(payload.request_id);
        if (!Number.isFinite(requestId)) return;
        const key = `${sessionId}:${requestId}`;
        if (kind === "ToolCallUpdated") instructionToolSessionsRef.current.add(sessionId);
        scheduleStreamUpdate((current) => {
          const next = cloneStream(current[key] ?? createStream(key, requestId));
          next.status = "streaming";
          next.retrying = undefined;
          next.providerError = undefined;
          if (kind === "ToolCallUpdated") {
            const toolCall = toolCallFromPayload(payload.tool_call);
            if (!toolCall) return current;
            const entry = ensureToolCall(next, toolCall);
            if (
              entry.status !== "running" &&
              entry.status !== "completed" &&
              entry.status !== "failed"
            ) {
              entry.status = "pending";
            }
          } else if (kind === "Delta") {
            appendSegment(next, "text", asString(payload.content));
          } else {
            const content = asString(payload.content);
            appendSegment(next, "reasoning", content);
            if (content) next.reasoningStartedAt ??= new Date().toISOString();
          }
          return { ...current, [key]: next };
        });
        return;
      }
      if (kind === "ToolStarting") {
        const requestId = Number(payload.request_id);
        const toolCall = toolCallFromPayload(payload.tool_call);
        if (!Number.isFinite(requestId) || !toolCall) return;
        const key = `${sessionId}:${requestId}`;
        instructionToolSessionsRef.current.add(sessionId);
        scheduleStreamUpdate((current) => {
          const next = cloneStream(current[key] ?? createStream(key, requestId));
          const entry = ensureToolCall(next, toolCall);
          if (entry.status !== "completed" && entry.status !== "failed") {
            entry.status = "running";
          }
          return { ...current, [key]: next };
        });
        return;
      }
      if (kind === "ToolCompleted" || kind === "SubagentCompleted") {
        const requestId = Number(payload.request_id);
        const toolCall = toolCallFromPayload(payload.tool_call);
        const result = toolResultFromPayload(payload.result);
        if (!Number.isFinite(requestId) || !toolCall || !result) return;
        const key = `${sessionId}:${requestId}`;
        const childSessionId = asString(payload.child_session_id);
        scheduleStreamUpdate((current) => {
          const next = cloneStream(current[key] ?? createStream(key, requestId));
          const entry = ensureToolCall(next, toolCall);
          next.toolCallMap[toolCall.id] = {
            ...entry,
            result,
            status: toolResultStatus(result),
            ...(childSessionId ? { childSessionId } : {}),
          };
          return { ...current, [key]: next };
        });
        return;
      }
      if (kind === "SubagentStatus") {
        const requestId = Number(payload.request_id);
        const toolCallId = asString(payload.tool_call_id);
        const childSessionId = asString(payload.child_session_id);
        if (!Number.isFinite(requestId) || !toolCallId || !childSessionId) return;
        const currentToolCall = toolCallFromPayload(payload.current_tool_call);
        const key = `${sessionId}:${requestId}`;
        scheduleStreamUpdate((current) => {
          const next = cloneStream(current[key] ?? createStream(key, requestId));
          if (!next.toolCallMap[toolCallId]) {
            ensureToolCall(
              next,
              currentToolCall ?? {
                id: toolCallId,
                name: "task",
                arguments: "{}",
                thought_signature: null,
              },
            );
          }
          updateSubagentEntry(
            next,
            toolCallId,
            childSessionId,
            asString(payload.status_text),
            currentToolCall,
            asString(payload.content_delta),
            asString(payload.reasoning_delta),
          );
          return { ...current, [key]: next };
        });
        return;
      }
      if (kind === "ShellOutput") {
        const toolCallId = asString(payload.tool_call_id);
        if (!toolCallId) return;
        const finished = payload.finished === true;
        const exitCode = typeof payload.exit_code === "number" ? payload.exit_code : null;
        const content = asString(payload.content);
        scheduleStreamUpdate((current) => {
          const keys = Object.keys(current).filter(
            (key) => key.startsWith(`${sessionId}:`) && current[key]?.toolCallMap[toolCallId],
          );
          const key = keys[keys.length - 1];
          if (!key) return current;
          const stream = current[key];
          if (!stream) return current;
          const next = cloneStream(stream);
          const entry = next.toolCallMap[toolCallId];
          if (!entry) return current;
          const updated = { ...entry };
          updateShellResult(updated, content, finished, exitCode);
          next.toolCallMap[toolCallId] = updated;
          return { ...current, [key]: next };
        });
        return;
      }
      if (kind === "Retrying") {
        const requestId = Number(payload.request_id);
        if (!Number.isFinite(requestId)) return;
        const key = `${sessionId}:${requestId}`;
        const attempt = Number(payload.attempt);
        const maxAttempts = Number(payload.max_attempts);
        const reason = asString(payload.reason);
        const retryAfterSecs =
          typeof payload.retry_after_secs === "number" ? payload.retry_after_secs : null;
        scheduleStreamUpdate((current) => {
          const next = cloneStream(current[key] ?? createStream(key, requestId));
          next.status = "streaming";
          next.providerFinished = false;
          next.error = undefined;
          next.retrying = {
            attempt: Number.isFinite(attempt) ? attempt : 1,
            maxAttempts: Number.isFinite(maxAttempts) ? maxAttempts : 1,
            reason,
            retryAfterSecs,
          };
          next.providerError = {
            message: reason || i18n.t("Retrying…"),
            retryable: true,
            request_id: requestId,
            user_message_id: next.userMessageId ?? null,
          };
          return { ...current, [key]: next };
        });
        touchSession(sessionId, true);
        return;
      }
      if (kind === "Failed") {
        const requestId = Number(payload.request_id);
        if (!Number.isFinite(requestId)) return;
        const key = `${sessionId}:${requestId}`;
        instructionToolSessionsRef.current.delete(sessionId);
        scheduleStreamUpdate((current) => {
          const next = cloneStream(current[key] ?? createStream(key, requestId));
          next.status = "failed";
          next.retrying = undefined;
          const providerError = providerErrorFromPayload(
            payload.error,
            payload.retryable,
            requestId,
            next.userMessageId ?? null,
          );
          next.providerError = providerError;
          next.error = providerError.message;
          return { ...current, [key]: next };
        });
        touchSession(sessionId, false);
        return;
      }
      if (kind === "Finished") {
        const requestId = Number(payload.request_id);
        if (!Number.isFinite(requestId)) return;
        const key = `${sessionId}:${requestId}`;
        const turn = (payload.turn ?? {}) as Record<string, unknown>;
        if (Array.isArray(turn.tool_calls) && turn.tool_calls.length > 0) {
          instructionToolSessionsRef.current.add(sessionId);
        }
        scheduleStreamUpdate((current) => {
          const next = cloneStream(current[key] ?? createStream(key, requestId));
          const turnCompletedAt = asString(turn.completed_at) || null;
          reconcileSegment(next, "text", asString(turn.content));
          reconcileSegment(next, "reasoning", asString(turn.reasoning));
          next.providerFinished = true;
          next.status = "streaming";
          next.reasoningStartedAt = asString(turn.reasoning_started_at) || next.reasoningStartedAt;
          next.reasoningCompletedAt =
            asString(turn.reasoning_completed_at) ||
            next.reasoningCompletedAt ||
            (next.reasoningStartedAt ? turnCompletedAt : null);
          if (Array.isArray(turn.tool_calls)) {
            for (const value of turn.tool_calls) {
              const toolCall = toolCallFromPayload(value);
              if (toolCall) ensureToolCall(next, toolCall);
            }
          }
          return { ...current, [key]: next };
        });
        return;
      }
      if (kind === "StreamEnd") {
        const requestId = Number(payload.request_id);
        if (!Number.isFinite(requestId)) return;
        const key = `${sessionId}:${requestId}`;
        instructionToolSessionsRef.current.delete(sessionId);
        scheduleStreamUpdate((current) => {
          const stream = current[key];
          if (!stream) return current;
          if (stream.providerFinished && stream.status !== "failed") {
            const next = { ...current };
            delete next[key];
            return next;
          }
          if (stream.status !== "streaming") return current;
          return {
            ...current,
            [key]: {
              ...stream,
              status: "interrupted",
              reasoningStartedAt:
                asString(payload.reasoning_started_at) || stream.reasoningStartedAt,
              reasoningCompletedAt:
                asString(payload.reasoning_completed_at) ||
                stream.reasoningCompletedAt ||
                (stream.reasoningStartedAt ? new Date().toISOString() : null),
              error: i18n.t("The turn was interrupted."),
            },
          };
        });
        touchSession(sessionId, false);
        if (selectedSessionRef.current === sessionId) {
          const instructionRevision = instructionNoticeRevisionRef.current;
          void loadMessages(sessionId).then((loaded) => {
            if (
              loaded &&
              selectedSessionRef.current === sessionId &&
              instructionNoticeRevisionRef.current === instructionRevision
            ) {
              clearPendingInstructionNotices();
            }
          });
          void loadTodos(sessionId);
        }
        return;
      }
      if (kind === "MessagesTruncated" && selectedSessionRef.current === sessionId) {
        void loadMessages(sessionId);
      }
    },
    [clearPendingInstructionNotices, loadMessages, loadTodos, scheduleStreamUpdate, touchSession],
  );

  useEffect(() => {
    if (authChecking || (authRequired && !authenticated)) return;
    const backend = openBackendEvents(
      cursorRef.current,
      applyEvent,
      () => {
        const currentSessionId = selectedSessionRef.current;
        if (currentSessionId) {
          void loadMessages(currentSessionId);
          void loadTodos(currentSessionId);
        }
      },
      () => undefined,
    );
    const approval = openFrontendRequests(
      (request) =>
        setRequests((current) => [
          ...current.filter((item) => item.request_id !== request.request_id),
          request,
        ]),
      () => undefined,
    );
    return () => {
      backend.close();
      approval.close();
    };
  }, [applyEvent, authChecking, authRequired, authenticated, loadMessages, loadTodos]);

  const renameSession = async (sessionId: string) => {
    const title = renameValue.trim();
    if (!title) {
      setRenamingSessionId(null);
      return;
    }
    try {
      const updated = await api.renameSession(sessionId, title);
      setSessions((current) =>
        current.map((item) => (item.session_id === sessionId ? updated : item)),
      );
      setSelectedSession((current) => (current?.session_id === sessionId ? updated : current));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to rename session"));
    } finally {
      setRenamingSessionId(null);
    }
  };

  const deleteSession = async (session: Session) => {
    if (
      !window.confirm(
        i18n.t("Delete conversation “{{title}}”?", {
          title: session.title || i18n.t("Untitled conversation"),
        }),
      )
    )
      return;
    try {
      await api.deleteSession(session.session_id);
      setSessions((current) => current.filter((item) => item.session_id !== session.session_id));
      setSelectedSession((current) =>
        current?.session_id === session.session_id ? undefined : current,
      );
      if (selectedSessionRef.current === session.session_id) createSession();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to delete session"));
    }
  };

  const submitWelcome = async (workspaceRoot: string) => {
    const content = draft.trim();
    const selectedWorkspaceRoot = workspaceRoot.trim();
    if (!content || !selectedWorkspaceRoot || welcomeSending) return;
    setWelcomeSending(true);
    setError(null);
    try {
      const response = await api.createSession({
        title: content.slice(0, 80),
        workspace_root: selectedWorkspaceRoot,
      });
      setSessionSearch("");
      setSessionWorkspaceRoot(null);
      setSessions((current) => mergeSessions([response.session], current));
      setSessionWorkspaceRoots((current) =>
        current.includes(response.session.workspace_root)
          ? current
          : [response.session.workspace_root, ...current],
      );
      setFocusComposerAfterWelcome(true);
      setScrollToBottomRequest((current) => current + 1);
      selectSession(response.session.session_id, response.session);
      setDraft("");
      await api.sendPrompt(
        response.session.session_id,
        content,
        mode,
        crypto.randomUUID(),
        thinkingLevel,
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to start conversation"));
    } finally {
      setWelcomeSending(false);
    }
  };

  const handleRevert = useCallback(
    async (messageId: string) => {
      const sessionId = selectedSessionRef.current;
      if (!sessionId) return;
      try {
        await api.revertToMessage(sessionId, messageId);
        await loadMessages(sessionId);
        await loadTodos(sessionId);
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : i18n.t("Failed to revert"));
      }
    },
    [loadMessages, loadTodos],
  );

  const handleRetryProviderError = useCallback(
    async (messageId: string) => {
      const sessionId = selectedSessionRef.current;
      if (!sessionId || sending) return;
      setError(null);
      setSending(true);
      clearPendingStreamUpdates();
      setStreams((current) => {
        const next = { ...current };
        for (const [key, stream] of Object.entries(next)) {
          if (
            key.startsWith(`${sessionId}:`) &&
            (stream.userMessageId === messageId || stream.status !== "streaming")
          ) {
            delete next[key];
          }
        }
        return next;
      });
      try {
        await api.retrySession(sessionId, messageId);
        await loadMessages(sessionId);
      } catch (reason) {
        setError(
          reason instanceof Error ? reason.message : i18n.t("Failed to retry provider request"),
        );
      } finally {
        setSending(false);
      }
    },
    [clearPendingStreamUpdates, loadMessages, sending],
  );

  const handleRedo = useCallback(async () => {
    const sessionId = selectedSessionRef.current;
    if (!sessionId) return;
    try {
      await api.redoSession(sessionId);
      await loadMessages(sessionId);
      await loadTodos(sessionId);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to redo"));
    }
  }, [loadMessages, loadTodos]);

  const handleFork = useCallback(
    async (messageId: string) => {
      const sessionId = selectedSessionRef.current;
      if (!sessionId) return;
      try {
        const forked = await api.forkSession(sessionId, messageId);
        setSessions((current) => mergeSessions([forked], current));
        setSessionWorkspaceRoots((current) =>
          current.includes(forked.workspace_root) ? current : [forked.workspace_root, ...current],
        );
        selectSession(forked.session_id, forked);
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : i18n.t("Failed to fork"));
      }
    },
    [selectSession],
  );

  const handleCompact = useCallback(async () => {
    const sessionId = selectedSessionRef.current;
    if (!sessionId) return;
    try {
      await api.compactSession(sessionId);
      setError(null);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to compact"));
    }
  }, []);

  const handleSlashCommand = useCallback(
    async (raw: string): Promise<boolean> => {
      const parsed = parseSlashCommand(raw);
      if (!parsed) return false;
      const { command, args } = parsed;
      if (command === "undo") {
        const lastUser = [...messages].reverse().find((r) => r.message.role === "user");
        if (lastUser) await handleRevert(lastUser.message.id);
        else setError(i18n.t("No user message to undo"));
        return true;
      }
      if (command === "redo") {
        await handleRedo();
        return true;
      }
      if (command === "compact") {
        await handleCompact();
        return true;
      }
      if (command === "fork") {
        const target = [...messages].reverse().find((r) => r.message.role === "user");
        if (target) await handleFork(target.message.id);
        else setError(i18n.t("No message to fork from"));
        return true;
      }
      if (command === "rename" && args) {
        const sid = selectedSessionRef.current;
        if (sid) {
          try {
            const updated = await api.renameSession(sid, args);
            setSessions((current) =>
              current.map((item) => (item.session_id === sid ? updated : item)),
            );
            setSelectedSession((current) => (current?.session_id === sid ? updated : current));
          } catch (reason) {
            setError(reason instanceof Error ? reason.message : i18n.t("Failed to rename"));
          }
        }
        return true;
      }
      if (command === "new") {
        createSession();
        return true;
      }
      if (command === "mcp") {
        useUIStore.getState().openSettingsPanel("mcp");
        return true;
      }
      if (command === "skills" || command === "skill") {
        useUIStore.getState().openSettingsPanel("skills");
        return true;
      }
      return false;
    },
    [messages, handleRevert, handleRedo, handleFork, handleCompact, createSession],
  );

  const submit = async () => {
    const raw = draft;
    const content = raw.trim();
    const sessionId = selectedSessionRef.current;
    if (!content || !sessionId || sending) return;
    setFileMention(null);
    // Intercept slash commands before sending as a prompt.
    if (content.startsWith("/")) {
      const handled = await handleSlashCommand(content);
      if (handled) {
        setDraft("");
        return;
      }
    }
    const messageId = crypto.randomUUID();
    setSending(true);
    setScrollToBottomRequest((current) => current + 1);
    setDraft("");
    try {
      await api.sendPrompt(sessionId, content, mode, messageId, thinkingLevel);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to send prompt"));
      setDraft(content);
    } finally {
      setSending(false);
    }
  };

  const chooseModel = async (model: Model) => {
    try {
      const selected = await api.selectModel(model.provider_id, model.model_id);
      setModels((current) =>
        current.map((item) => ({
          ...item,
          active: item.provider_id === selected.provider_id && item.model_id === selected.model_id,
          thinking_level:
            item.provider_id === selected.provider_id && item.model_id === selected.model_id
              ? selected.thinking_level
              : item.thinking_level,
        })),
      );
      setThinkingLevel(selected.thinking_level);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to select model"));
    }
  };

  const chooseThinkingLevel = async (level: string) => {
    if (!activeModel) return;
    setThinkingLevel(level);
    try {
      await api.setThinkingLevel(activeModel.provider_id, activeModel.model_id, level);
      setModels((current) =>
        current.map((item) => (item.active ? { ...item, thinking_level: level } : item)),
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to set thinking level"));
    }
  };

  const respondToRequest = useCallback(async (requestId: string, tools: ApprovedTool[]) => {
    try {
      await api.respondToRequest(requestId, tools);
      setRequests((current) => current.filter((item) => item.request_id !== requestId));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to respond"));
    }
  }, []);

  const cancelSession = useCallback(async (sessionId: string) => {
    if (!sessionId?.trim()) return;
    setCanceling(true);
    try {
      await api.abortRequest(sessionId, { request_id: 0 });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to cancel"));
    } finally {
      setCanceling(false);
    }
  }, []);

  const visibleStreams = useMemo(
    () => Object.values(streams).filter((item) => item.key.startsWith(`${selectedSessionId}:`)),
    [selectedSessionId, streams],
  );
  const activeModel =
    models.find((model) => model.active) ??
    models.find(
      (model) =>
        model.model_id === selectedSession?.model_id &&
        model.provider_id === selectedSession?.provider_id,
    );

  return {
    authChecking,
    authRequired,
    authenticated,
    openSettingsPanel,
    enterToSend,
    sessions,
    sessionWorkspaceRoots,
    sessionWorkspaceRoot,
    nextSessionCursor,
    loadingMoreSessions,
    selectedSessionId,
    selectedSession,
    activeModel,
    messages,
    visibleStreams,
    requests,
    models,
    todos,
    draft,
    mode,
    loading,
    sending,
    canceling,
    welcomeSending,
    focusComposerAfterWelcome,
    clearComposerFocusRequest,
    scrollToBottomRequest,
    thinkingLevel,
    sessionSearch,
    renamingSessionId,
    renameValue,
    error,
    fileMention,
    fileMentionIndex,
    setDraft,
    setMode,
    setSessionSearch,
    setSessionWorkspaceRoot,
    setRenamingSessionId,
    setRenameValue,
    setFileMentionIndex,
    setFileMention,
    selectSession,
    createSession,
    loadMoreSessions,
    renameSession,
    deleteSession,
    submitWelcome,
    handleRevert,
    handleRetryProviderError,
    handleRedo,
    handleFork,
    handleCompact,
    submit,
    chooseModel,
    chooseThinkingLevel,
    respondToRequest,
    cancelSession,
    updateFileMention,
    handleFileSelect,
    instructionNotices,
  };
}
