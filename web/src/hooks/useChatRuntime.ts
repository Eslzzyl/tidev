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
  Session,
  TodoItem,
  ToolCall,
} from "../types/api";
import type { StreamMessage } from "../types/chat";
import { parseSlashCommand } from "../commands";
import { asString, eventPayload } from "../utils/events";
import i18n from "../i18n";

export function useChatRuntime() {
  const authChecking = useAuthStore((state) => state.isLoading);
  const authRequired = useAuthStore((state) => state.isAuthRequired);
  const authenticated = useAuthStore((state) => state.isAuthenticated);
  const checkAuthStatus = useAuthStore((state) => state.checkAuthStatus);
  const openSettingsPanel = useUIStore((state) => state.openSettingsPanel);
  const enterToSend = useUIStore((state) => state.settings.enterToSend);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<MessageRecord[]>([]);
  const [streams, setStreams] = useState<Record<string, StreamMessage>>({});
  const [requests, setRequests] = useState<FrontendRequest[]>([]);
  const [models, setModels] = useState<Model[]>([]);
  const [todos, setTodos] = useState<TodoItem[]>([]);
  const [draft, setDraft] = useState("");
  const [mode, setMode] = useState<"build" | "plan">("build");
  const [loading, setLoading] = useState(true);
  const [sending, setSending] = useState(false);
  const [canceling, setCanceling] = useState(false);
  const [welcomeSending, setWelcomeSending] = useState(false);
  const [thinkingLevel, setThinkingLevel] = useState<string | undefined>();
  const [sessionSearch, setSessionSearch] = useState("");
  const [renamingSessionId, setRenamingSessionId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [fileMention, setFileMention] = useState<{ query: string; atPos: number } | null>(null);
  const [fileMentionIndex, setFileMentionIndex] = useState(0);
  const selectedSessionRef = useRef<string | null>(null);
  const cursorRef = useRef<number | null>(
    Number(localStorage.getItem("tidev:last-event-cursor")) || null,
  );

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

  const loadMessages = useCallback(async (sessionId: string) => {
    try {
      const response = await api.listMessages(sessionId);
      if (selectedSessionRef.current === sessionId) {
        setMessages(response.messages);
      }
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to load messages"));
    }
  }, []);

  const loadTodos = useCallback(async (sessionId: string) => {
    try {
      const response = await api.getTodos(sessionId);
      if (selectedSessionRef.current === sessionId) setTodos(response.todos);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to load to-do list"));
    }
  }, []);

  const selectSession = useCallback(
    (sessionId: string) => {
      selectedSessionRef.current = sessionId;
      setSelectedSessionId(sessionId);
      setStreams({});
      setTodos([]);
      setError(null);
      setFileMention(null);
      void loadMessages(sessionId);
      void loadTodos(sessionId);
    },
    [loadMessages, loadTodos],
  );

  useEffect(() => {
    if (authChecking || (authRequired && !authenticated)) return;
    let disposed = false;
    void api
      .listSessions()
      .then(async (items) => {
        if (disposed) return;
        setSessions(items);
      })
      .catch((reason) =>
        setError(reason instanceof Error ? reason.message : i18n.t("Failed to load sessions")),
      )
      .finally(() => setLoading(false));
    return () => {
      disposed = true;
    };
  }, [authChecking, authRequired, authenticated]);

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
        if (message && payload.queued !== true && selectedSessionRef.current === sessionId) {
          setMessages((current) =>
            current.some((item) => item.message.id === message.id)
              ? current
              : [...current, { message, app_data: appData ?? {} }],
          );
        }
        return;
      }
      if (kind === "TurnStarting") {
        const requestId = Number(payload.request_id);
        if (Number.isFinite(requestId)) {
          const key = `${sessionId}:${requestId}`;
          setStreams((current) => {
            if (!current[key]) return current;
            const next = { ...current };
            delete next[key];
            return next;
          });
        }
        setSessions((current) =>
          current.map((item) => (item.session_id === sessionId ? { ...item, busy: true } : item)),
        );
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
        setStreams((current) => {
          const previous = current[key] ?? {
            key,
            requestId,
            content: "",
            reasoning: "",
            toolCalls: [],
            status: "streaming" as const,
          };
          const next = { ...previous };
          if (kind === "Delta") next.content += asString(payload.content);
          if (kind === "ReasoningDelta" || kind === "ReasoningSummaryDelta")
            next.reasoning += asString(payload.content);
          if (kind === "ToolCallUpdated") {
            const toolCall = payload.tool_call as ToolCall;
            next.toolCalls = [
              ...next.toolCalls.filter((item) => item.id !== toolCall.id),
              toolCall,
            ];
          }
          return { ...current, [key]: next };
        });
        return;
      }
      if (kind === "Failed") {
        const requestId = Number(payload.request_id);
        const key = `${sessionId}:${requestId}`;
        setStreams((current) => ({
          ...current,
          [key]: {
            ...(current[key] ?? { key, requestId, content: "", reasoning: "", toolCalls: [] }),
            status: "failed",
            error: asString(payload.error),
          },
        }));
        setSessions((current) =>
          current.map((item) => (item.session_id === sessionId ? { ...item, busy: false } : item)),
        );
        return;
      }
      if (kind === "Finished") {
        const requestId = Number(payload.request_id);
        const key = `${sessionId}:${requestId}`;
        setStreams((current) => {
          if (current[key] && current[key].status !== "streaming") return current;
          const next = { ...current };
          delete next[key];
          return next;
        });
        setSessions((current) =>
          current.map((item) => (item.session_id === sessionId ? { ...item, busy: false } : item)),
        );
        if (selectedSessionRef.current === sessionId) {
          void loadMessages(sessionId);
          void loadTodos(sessionId);
        }
        return;
      }
      if (kind === "StreamEnd") {
        const requestId = Number(payload.request_id);
        const key = `${sessionId}:${requestId}`;
        setStreams((current) => {
          const stream = current[key];
          if (!stream || stream.status !== "streaming") return current;
          return {
            ...current,
            [key]: {
              ...stream,
              status: "interrupted",
              error: i18n.t("The turn was interrupted."),
            },
          };
        });
        setSessions((current) =>
          current.map((item) => (item.session_id === sessionId ? { ...item, busy: false } : item)),
        );
        if (selectedSessionRef.current === sessionId) {
          void loadMessages(sessionId);
          void loadTodos(sessionId);
        }
        return;
      }
      if (kind === "MessagesTruncated" && selectedSessionRef.current === sessionId) {
        void loadMessages(sessionId);
      }
    },
    [loadMessages, loadTodos],
  );

  useEffect(() => {
    if (authChecking || (authRequired && !authenticated)) return;
    const backend = openBackendEvents(
      cursorRef.current,
      applyEvent,
      () => void loadMessages(selectedSessionRef.current ?? ""),
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
  }, [applyEvent, authChecking, authRequired, authenticated, loadMessages]);

  const createSession = () => {
    selectedSessionRef.current = null;
    setSelectedSessionId(null);
    setMessages([]);
    setStreams({});
    setDraft("");
    setFileMention(null);
    setError(null);
  };

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
      if (selectedSessionRef.current === session.session_id) createSession();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : i18n.t("Failed to delete session"));
    }
  };

  const submitWelcome = async () => {
    const content = draft.trim();
    if (!content || welcomeSending) return;
    setWelcomeSending(true);
    setError(null);
    try {
      const response = await api.createSession(content.slice(0, 80));
      setSessions((current) => [response.session, ...current]);
      selectSession(response.session.session_id);
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
        setSessions((current) => [forked, ...current]);
        selectSession(forked.session_id);
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

  const handleShell = useCallback(
    async (command: string) => {
      const sessionId = selectedSessionRef.current;
      if (!sessionId || !command.trim()) return;
      try {
        await api.sendShellCommand(sessionId, command);
        // Shell output will appear via subsequent message reload triggered by
        // the shell task appending messages; poll briefly.
        setTimeout(() => void loadMessages(sessionId), 400);
        setTimeout(() => void loadMessages(sessionId), 1200);
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : i18n.t("Failed to run shell command"));
      }
    },
    [loadMessages],
  );

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
      if (command === "shell") {
        if (!args) {
          setError(i18n.t("Usage: /shell <command> or !<command>"));
          return true;
        }
        await handleShell(args);
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
      return false;
    },
    [messages, handleRevert, handleRedo, handleFork, handleCompact, handleShell],
  );

  const submit = async () => {
    const raw = draft;
    const content = raw.trim();
    const sessionId = selectedSessionRef.current;
    if (!content || !sessionId || sending) return;
    setFileMention(null);
    // Intercept slash / bang commands before sending as a prompt.
    if (content.startsWith("/") || content.startsWith("!")) {
      const handled = await handleSlashCommand(content);
      if (handled) {
        setDraft("");
        return;
      }
    }
    const messageId = crypto.randomUUID();
    setSending(true);
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
  const selectedSession = sessions.find((session) => session.session_id === selectedSessionId);
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
    setRenamingSessionId,
    setRenameValue,
    setFileMentionIndex,
    setFileMention,
    selectSession,
    createSession,
    renameSession,
    deleteSession,
    submitWelcome,
    handleRevert,
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
  };
}
