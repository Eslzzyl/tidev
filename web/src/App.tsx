import { type FormEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  BarChart3,
  Check,
  ChevronDown,
  CircleStop,
  Clock3,
  FolderTree,
  GitBranch,
  ListTodo,
  LoaderCircle,
  MessageSquare,
  Pencil,
  Plus,
  Search,
  Settings,
  Send,
  Sparkles,
  Terminal,
  X,
  Trash2,
} from "lucide-react";

import { api, getAuthToken, openBackendEvents, openFrontendRequests, setAuthToken } from "./api";
import SettingsPanel from "./SettingsPanel";
import type {
  ApprovedTool,
  EventEnvelope,
  Feature,
  FrontendRequest,
  Message,
  MessageRecord,
  Model,
  Session,
  StreamMessage,
  TodoItem,
  ToolCall,
} from "./types";

const features: { id: Feature; label: string; icon: typeof MessageSquare }[] = [
  { id: "chat", label: "Chat", icon: MessageSquare },
  { id: "files", label: "Files", icon: FolderTree },
  { id: "terminal", label: "Terminal", icon: Terminal },
  { id: "git", label: "Git", icon: GitBranch },
  { id: "stats", label: "Stats", icon: BarChart3 },
];

function eventPayload(envelope: EventEnvelope): [string, Record<string, unknown>] {
  const entry = Object.entries(envelope.event)[0];
  return entry ? [entry[0], (entry[1] ?? {}) as Record<string, unknown>] : ["", {}];
}

function asString(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function roleLabel(role: Message["role"]): string {
  switch (role) {
    case "user":
      return "You";
    case "assistant":
      return "Assistant";
    case "tool":
      return "Tool";
    case "system":
      return "System";
    case "shell":
      return "Shell";
    case "error":
      return "Error";
  }
}

function makeRejectedTool(tool: ToolCall): ApprovedTool {
  return {
    tool_call: tool,
    rejection: {
      output: "The user rejected this tool call.",
      attachments: [],
      metadata: {},
    },
    child_session_id: null,
    allow_outside: false,
    sensitive_file_approved: false,
    user_reason: "Rejected in Web UI",
  };
}

function makeApprovedTool(tool: ToolCall): ApprovedTool {
  return {
    tool_call: tool,
    rejection: null,
    child_session_id: null,
    allow_outside: true,
    sensitive_file_approved: true,
    user_reason: null,
  };
}

export default function App() {
  const [feature, setFeature] = useState<Feature>("chat");
  const [authChecking, setAuthChecking] = useState(true);
  const [authRequired, setAuthRequired] = useState(false);
  const [authenticated, setAuthenticated] = useState(false);
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
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const [thinkingPickerOpen, setThinkingPickerOpen] = useState(false);
  const [todoPickerOpen, setTodoPickerOpen] = useState(false);
  const [thinkingLevel, setThinkingLevel] = useState<string | undefined>();
  const [enterToSend, setEnterToSend] = useState(() => readEnterToSendPreference());
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [sessionSearch, setSessionSearch] = useState("");
  const [renamingSessionId, setRenamingSessionId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const selectedSessionRef = useRef<string | null>(null);
  const composerTextareaRef = useRef<HTMLTextAreaElement>(null);
  const composingRef = useRef(false);
  const compositionJustCommittedRef = useRef(false);
  const compositionEndTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const cursorRef = useRef<number | null>(
    Number(localStorage.getItem("tidev:last-event-cursor")) || null,
  );

  useEffect(() => {
    let disposed = false;
    void api
      .authStatus()
      .then(async ({ auth_required }) => {
        if (disposed) return;
        setAuthRequired(auth_required);
        if (!auth_required) {
          setAuthenticated(true);
          return;
        }
        const token = getAuthToken();
        const valid = token ? (await api.verifyAuthToken(token)).valid : false;
        if (disposed) return;
        if (!valid) setAuthToken("");
        setAuthenticated(valid);
      })
      .catch((reason) => {
        if (!disposed) {
          setError(reason instanceof Error ? reason.message : "Failed to contact tidev");
          setAuthenticated(true);
        }
      })
      .finally(() => {
        if (!disposed) setAuthChecking(false);
      });
    return () => {
      disposed = true;
    };
  }, []);

  useEffect(() => {
    const textarea = composerTextareaRef.current;
    if (!textarea) return;
    textarea.style.height = "auto";
    const height = Math.min(textarea.scrollHeight, 200);
    textarea.style.height = `${height}px`;
    textarea.style.overflowY = textarea.scrollHeight > 200 ? "auto" : "hidden";
  }, [draft]);

  useEffect(() => () => {
    if (compositionEndTimerRef.current) clearTimeout(compositionEndTimerRef.current);
  });

  const loadMessages = useCallback(async (sessionId: string) => {
    try {
      const response = await api.messages(sessionId);
      if (selectedSessionRef.current === sessionId) {
        setMessages(response.messages);
      }
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Failed to load messages");
    }
  }, []);

  const loadTodos = useCallback(async (sessionId: string) => {
    try {
      const response = await api.todos(sessionId);
      if (selectedSessionRef.current === sessionId) setTodos(response.todos);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Failed to load to-do list");
    }
  }, []);

  const selectSession = useCallback(
    (sessionId: string) => {
      selectedSessionRef.current = sessionId;
      setSelectedSessionId(sessionId);
      setStreams({});
      setTodos([]);
      setError(null);
      void loadMessages(sessionId);
      void loadTodos(sessionId);
    },
    [loadMessages, loadTodos],
  );

  useEffect(() => {
    if (authChecking || (authRequired && !authenticated)) return;
    let disposed = false;
    void api
      .sessions()
      .then(async (items) => {
        if (disposed) return;
        setSessions(items);
      })
      .catch((reason) =>
        setError(reason instanceof Error ? reason.message : "Failed to load sessions"),
      )
      .finally(() => setLoading(false));
    return () => {
      disposed = true;
    };
  }, [authChecking, authRequired, authenticated]);

  const refreshModels = useCallback(async () => {
    try {
      const available = await api.models();
      setModels(available);
      setThinkingLevel(
        (current) => current ?? available.find((model) => model.active)?.thinking_level,
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Failed to load models");
    }
  }, []);

  useEffect(() => {
    void refreshModels();
  }, [refreshModels]);

  useEffect(() => {
    const handleSettings = (event: Event) => {
      const detail = (event as CustomEvent<{ enterToSend?: boolean }>).detail;
      if (typeof detail?.enterToSend === "boolean") setEnterToSend(detail.enterToSend);
    };
    window.addEventListener("tidev-ui-settings-changed", handleSettings);
    return () => window.removeEventListener("tidev-ui-settings-changed", handleSettings);
  }, []);

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
      if (kind === "Finished" || kind === "StreamEnd") {
        const requestId = Number(payload.request_id);
        const key = `${sessionId}:${requestId}`;
        setStreams((current) => {
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
    setError(null);
  };

  const renameSession = async (sessionId: string) => {
    const title = renameValue.trim();
    if (!title) {
      setRenamingSessionId(null);
      return;
    }
    try {
      const updated = await api.updateSession(sessionId, title);
      setSessions((current) =>
        current.map((item) => (item.session_id === sessionId ? updated : item)),
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Failed to rename session");
    } finally {
      setRenamingSessionId(null);
    }
  };

  const deleteSession = async (session: Session) => {
    if (!window.confirm(`Delete conversation “${session.title || "Untitled"}”?`)) return;
    try {
      await api.deleteSession(session.session_id);
      setSessions((current) => current.filter((item) => item.session_id !== session.session_id));
      if (selectedSessionRef.current === session.session_id) createSession();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Failed to delete session");
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
      setError(reason instanceof Error ? reason.message : "Failed to start conversation");
    } finally {
      setWelcomeSending(false);
    }
  };

  const submit = async () => {
    const content = draft.trim();
    const sessionId = selectedSessionRef.current;
    if (!content || !sessionId || sending) return;
    const messageId = crypto.randomUUID();
    setSending(true);
    setDraft("");
    try {
      await api.sendPrompt(sessionId, content, mode, messageId, thinkingLevel);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Failed to send prompt");
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
      setModelPickerOpen(false);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Failed to select model");
    }
  };

  const chooseThinkingLevel = async (level: string) => {
    if (!activeModel) return;
    setThinkingLevel(level);
    setThinkingPickerOpen(false);
    try {
      await api.setThinkingLevel(activeModel.provider_id, activeModel.model_id, level);
      setModels((current) =>
        current.map((item) => (item.active ? { ...item, thinking_level: level } : item)),
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Failed to set thinking level");
    }
  };

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
  const isBusy = selectedSession?.busy ?? false;
  const pendingRequests = requests.filter((request) => request.session_id === selectedSessionId);

  if (authChecking) {
    return <AuthLoading />;
  }

  if (authRequired && !authenticated) {
    return (
      <AuthGate
        onAuthenticated={() => {
          setAuthenticated(true);
          setError(null);
        }}
      />
    );
  }

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="brand-mark">
          <span className="brand-glyph">t</span>
          <span>tidev</span>
        </div>
        <nav className="feature-nav" aria-label="Primary">
          {features.map(({ id, label, icon: Icon }) => (
            <button
              className={feature === id ? "feature-link active" : "feature-link"}
              key={id}
              onClick={() => setFeature(id)}
            >
              <Icon size={16} strokeWidth={1.8} />
              {label}
            </button>
          ))}
        </nav>
        <div className="header-title">
          {feature === "chat"
            ? (selectedSession?.title ?? "Chat")
            : features.find((item) => item.id === feature)?.label}
          {feature === "chat" && activeModel ? (
            <span>· {activeModel.model_display_name}</span>
          ) : null}
        </div>
        <div className="topbar-meta">
          <span className="connection-dot" />
          <span>Local runtime</span>
          <button
            className="settings-button"
            onClick={() => setSettingsOpen(true)}
            aria-label="Settings"
            title="Settings"
          >
            <Settings size={16} />
          </button>
        </div>
      </header>

      <main className="workspace">
        {feature === "chat" ? (
          selectedSessionId === null ? (
            <WelcomePage
              draft={draft}
              error={error}
              loading={loading}
              mode={mode}
              enterToSend={enterToSend}
              sending={welcomeSending}
              sessions={sessions}
              models={models}
              activeModel={activeModel}
              thinkingLevel={thinkingLevel}
              onChangeDraft={setDraft}
              onModeChange={setMode}
              onSelectSession={selectSession}
              onSelectModel={(model) => void chooseModel(model)}
              onSelectThinkingLevel={(level) => void chooseThinkingLevel(level)}
              onSubmit={() => void submitWelcome()}
            />
          ) : (
            <>
              <aside className="session-sidebar">
                <div className="sidebar-heading">
                  <div>
                    <span className="eyebrow">Workspace</span>
                    <strong>Conversations</strong>
                  </div>
                  <button
                    className="icon-button"
                    onClick={() => void createSession()}
                    title="New conversation"
                  >
                    <Plus size={17} />
                  </button>
                </div>
                <div className="session-search">
                  <Search size={14} />
                  <input
                    value={sessionSearch}
                    onChange={(event) => setSessionSearch(event.target.value)}
                    placeholder="Search sessions…"
                    aria-label="Search sessions"
                  />
                </div>
                <div className="session-list">
                  {loading ? <div className="empty-state">Loading sessions…</div> : null}
                  {!loading && sessions.length === 0 ? (
                    <div className="empty-state">No conversations yet.</div>
                  ) : null}
                  {sessions
                    .filter((session) =>
                      session.title.toLowerCase().includes(sessionSearch.trim().toLowerCase()),
                    )
                    .map((session) => (
                      <div
                        className={
                          selectedSessionId === session.session_id
                            ? "session-item selected"
                            : "session-item"
                        }
                        key={session.session_id}
                      >
                        {renamingSessionId === session.session_id ? (
                          <input
                            className="session-rename-input"
                            value={renameValue}
                            onChange={(event) => setRenameValue(event.target.value)}
                            onBlur={() => void renameSession(session.session_id)}
                            onKeyDown={(event) => {
                              if (event.key === "Enter") void renameSession(session.session_id);
                              if (event.key === "Escape") setRenamingSessionId(null);
                            }}
                            autoFocus
                          />
                        ) : (
                          <button
                            className="session-select"
                            onClick={() => selectSession(session.session_id)}
                            onDoubleClick={() => {
                              setRenamingSessionId(session.session_id);
                              setRenameValue(session.title);
                            }}
                          >
                            <span className="session-title">
                              {session.title || "Untitled conversation"}
                            </span>
                            <span className="session-meta">
                              {session.busy ? <span className="busy-indicator" /> : null}
                              {session.model_display_name} · {formatDate(session.updated_at)}
                            </span>
                          </button>
                        )}
                        {renamingSessionId !== session.session_id ? (
                          <span className="session-actions">
                            <button
                              onClick={() => {
                                setRenamingSessionId(session.session_id);
                                setRenameValue(session.title);
                              }}
                              title="Rename conversation"
                              aria-label="Rename conversation"
                            >
                              <Pencil size={13} />
                            </button>
                            <button
                              onClick={() => void deleteSession(session)}
                              title="Delete conversation"
                              aria-label="Delete conversation"
                            >
                              <Trash2 size={13} />
                            </button>
                          </span>
                        ) : null}
                      </div>
                    ))}
                </div>
                <div className="sidebar-footer">
                  <span>{sessions.length} conversations</span>
                  <span className="workspace-path" title={sessions[0]?.workspace_root}>
                    {shortPath(sessions[0]?.workspace_root ?? "")}
                  </span>
                </div>
              </aside>
              <section className="chat-panel">
                <div className="panel-header">
                  <div>
                    <span className="eyebrow">Conversation</span>
                    <h1>{selectedSession?.title ?? "New conversation"}</h1>
                  </div>
                  <div className="panel-actions">
                    <span className="model-label">
                      <Sparkles size={15} />
                      {selectedSession?.model_display_name ?? "Runtime model"}
                    </span>
                  </div>
                </div>
                <div className="message-stage">
                  <VirtualMessageList messages={messages} streams={visibleStreams} />
                  {pendingRequests.map((request) => (
                    <ApprovalCard
                      key={request.request_id}
                      request={request}
                      onRespond={(tools) => {
                        void api
                          .respondToRequest(request.request_id, tools)
                          .then(() =>
                            setRequests((current) =>
                              current.filter((item) => item.request_id !== request.request_id),
                            ),
                          )
                          .catch((reason) =>
                            setError(
                              reason instanceof Error ? reason.message : "Failed to respond",
                            ),
                          );
                      }}
                    />
                  ))}
                  {error ? <div className="error-banner">{error}</div> : null}
                </div>
                <div className="composer-wrap">
                  <div className="composer-toolbar">
                    <button
                      className={
                        mode === "plan" ? "composer-control plan" : "composer-control build"
                      }
                      onClick={() => setMode((current) => (current === "plan" ? "build" : "plan"))}
                    >
                      {mode === "plan" ? "Plan" : "Build"}
                    </button>
                    <div className="composer-menu">
                      <button
                        className="composer-control neutral"
                        onClick={() => {
                          setModelPickerOpen((current) => !current);
                          setThinkingPickerOpen(false);
                          setTodoPickerOpen(false);
                        }}
                      >
                        <span>
                          {activeModel
                            ? `${activeModel.provider_display_name}/${activeModel.model_display_name}`
                            : "Select model"}
                        </span>
                        <ChevronDown size={13} />
                      </button>
                      {modelPickerOpen ? (
                        <div className="composer-popover model-popover">
                          {models.map((model) => (
                            <button
                              key={`${model.provider_id}:${model.model_id}`}
                              className={
                                model.active ? "composer-option selected" : "composer-option"
                              }
                              disabled={!model.connected}
                              onClick={() => {
                                void chooseModel(model);
                              }}
                            >
                              <span>
                                {model.provider_display_name}/{model.model_display_name}
                              </span>
                              <small>{model.connected ? "Connected" : "Not connected"}</small>
                            </button>
                          ))}
                        </div>
                      ) : null}
                    </div>
                    {activeModel?.thinking_levels.length ? (
                      <div className="composer-menu">
                        <button
                          className="composer-control thinking"
                          onClick={() => {
                            setThinkingPickerOpen((current) => !current);
                            setModelPickerOpen(false);
                            setTodoPickerOpen(false);
                          }}
                        >
                          <span>
                            {formatThinkingLevel(thinkingLevel ?? activeModel.thinking_level)}
                          </span>
                          <ChevronDown size={13} />
                        </button>
                        {thinkingPickerOpen ? (
                          <div className="composer-popover thinking-popover">
                            {activeModel.thinking_levels.map((level) => (
                              <button
                                key={level}
                                className={
                                  thinkingLevel === level
                                    ? "composer-option selected"
                                    : "composer-option"
                                }
                                onClick={() => {
                                  void chooseThinkingLevel(level);
                                }}
                              >
                                {formatThinkingLevel(level)}
                              </button>
                            ))}
                          </div>
                        ) : null}
                      </div>
                    ) : null}
                    <div className="composer-menu">
                      <button
                        className="composer-control neutral"
                        onClick={() => {
                          setTodoPickerOpen((current) => !current);
                          setModelPickerOpen(false);
                          setThinkingPickerOpen(false);
                        }}
                      >
                        <ListTodo size={13} />
                        <span>To-Do{todos.length ? ` (${todos.length})` : ""}</span>
                        <ChevronDown size={13} />
                      </button>
                      {todoPickerOpen ? (
                        <div className="composer-popover todo-popover">
                          {todos.length ? (
                            todos.map((todo, index) => (
                              <div className="todo-item" key={`${todo.content}:${index}`}>
                                <span
                                  className={
                                    todo.status === "completed" ? "todo-check done" : "todo-check"
                                  }
                                >
                                  {todo.status === "completed" ? <Check size={11} /> : null}
                                </span>
                                <span>{todo.content}</span>
                              </div>
                            ))
                          ) : (
                            <div className="todo-empty">No to-do items in this conversation.</div>
                          )}
                        </div>
                      ) : null}
                    </div>
                    <div className="composer-spacer" />
                    <span className="composer-hint">
                      {enterToSend
                        ? "Enter to send · Shift+Enter for newline"
                        : "Ctrl+Enter to send"}
                    </span>
                  </div>
                  <div className="composer">
                    <textarea
                      ref={composerTextareaRef}
                      value={draft}
                      onChange={(event) => setDraft(event.target.value)}
                      onCompositionStart={() => {
                        composingRef.current = true;
                        compositionJustCommittedRef.current = false;
                        if (compositionEndTimerRef.current)
                          clearTimeout(compositionEndTimerRef.current);
                      }}
                      onCompositionEnd={() => {
                        composingRef.current = false;
                        compositionJustCommittedRef.current = true;
                        if (compositionEndTimerRef.current)
                          clearTimeout(compositionEndTimerRef.current);
                        compositionEndTimerRef.current = setTimeout(() => {
                          compositionJustCommittedRef.current = false;
                        }, 0);
                      }}
                      onKeyDown={(event) => {
                        if (event.key === "Tab") {
                          event.preventDefault();
                          setMode((current) => (current === "plan" ? "build" : "plan"));
                          return;
                        }
                        if (
                          event.key === "Enter" &&
                          !event.nativeEvent.isComposing &&
                          !composingRef.current &&
                          !compositionJustCommittedRef.current &&
                          ((enterToSend && !event.shiftKey) ||
                            (!enterToSend && (event.ctrlKey || event.metaKey)))
                        ) {
                          event.preventDefault();
                          void submit();
                        }
                      }}
                      placeholder="Ask tidev to inspect, plan, or change your workspace…"
                      rows={3}
                    />
                    <button
                      className={isBusy ? "send-button stop" : "send-button"}
                      disabled={
                        sending || canceling || !selectedSessionId || (!isBusy && !draft.trim())
                      }
                      onClick={() => {
                        if (!selectedSessionId) return;
                        if (isBusy) {
                          setCanceling(true);
                          void api
                            .cancel(selectedSessionId)
                            .catch((reason) =>
                              setError(
                                reason instanceof Error ? reason.message : "Failed to cancel",
                              ),
                            )
                            .finally(() => setCanceling(false));
                        } else {
                          void submit();
                        }
                      }}
                      title={isBusy ? "Stop current turn" : "Send prompt"}
                    >
                      {sending || canceling ? (
                        <LoaderCircle className="spin" size={17} />
                      ) : isBusy ? (
                        <CircleStop size={17} />
                      ) : (
                        <Send size={17} />
                      )}
                    </button>
                  </div>
                </div>
              </section>
            </>
          )
        ) : (
          <DeferredFeature feature={feature} />
        )}
      </main>
      <SettingsPanel open={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </div>
  );
}

function AuthLoading() {
  return (
    <main className="auth-page">
      <div className="auth-card">
        <div className="welcome-logo">t</div>
        <h1>tidev</h1>
        <p>Connecting to the local runtime…</p>
      </div>
    </main>
  );
}

function AuthGate({ onAuthenticated }: { onAuthenticated: () => void }) {
  const [token, setToken] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!token || submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      const response = await api.verifyAuthToken(token);
      if (!response.valid) throw new Error("The password is incorrect.");
      setAuthToken(token);
      onAuthenticated();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Unable to verify password.");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <main className="auth-page">
      <form className="auth-card" onSubmit={(event) => void submit(event)}>
        <div className="welcome-logo">t</div>
        <h1>tidev</h1>
        <p>This local tidev server requires its web access password.</p>
        <label>
          Password
          <input
            type="password"
            value={token}
            onChange={(event) => setToken(event.target.value)}
            autoFocus
            autoComplete="current-password"
          />
        </label>
        {error ? <div className="auth-error">{error}</div> : null}
        <button className="settings-primary" disabled={submitting || !token} type="submit">
          {submitting ? "Checking…" : "Open tidev"}
        </button>
      </form>
    </main>
  );
}

function WelcomePage({
  draft,
  error,
  loading,
  mode,
  enterToSend,
  sending,
  sessions,
  models,
  activeModel,
  thinkingLevel,
  onChangeDraft,
  onModeChange,
  onSelectSession,
  onSelectModel,
  onSelectThinkingLevel,
  onSubmit,
}: {
  draft: string;
  error: string | null;
  loading: boolean;
  mode: "build" | "plan";
  enterToSend: boolean;
  sending: boolean;
  sessions: Session[];
  models: Model[];
  activeModel: Model | undefined;
  thinkingLevel: string | undefined;
  onChangeDraft: (value: string) => void;
  onModeChange: (mode: "build" | "plan") => void;
  onSelectSession: (sessionId: string) => void;
  onSelectModel: (model: Model) => void;
  onSelectThinkingLevel: (level: string) => void;
  onSubmit: () => void;
}) {
  const compositionRef = useRef(false);
  const [modelOpen, setModelOpen] = useState(false);
  const [thinkingOpen, setThinkingOpen] = useState(false);
  return (
    <section className="welcome-page">
      <div className="welcome-heading">
        <div className="welcome-logo">t</div>
        <h1>tidev</h1>
        <p>Your intelligent coding assistant</p>
      </div>
      <div className="welcome-composer">
        <textarea
          value={draft}
          onChange={(event) => onChangeDraft(event.target.value)}
          onCompositionStart={() => {
            compositionRef.current = true;
          }}
          onCompositionEnd={() => {
            compositionRef.current = false;
          }}
          onKeyDown={(event) => {
            if (
              event.key === "Enter" &&
              !event.nativeEvent.isComposing &&
              !compositionRef.current &&
              ((enterToSend && !event.shiftKey) ||
                (!enterToSend && (event.ctrlKey || event.metaKey)))
            ) {
              event.preventDefault();
              onSubmit();
            }
          }}
          autoFocus
          disabled={loading || sending}
          placeholder="What would you like to work on?"
          rows={3}
        />
        <div className="welcome-composer-footer">
          <div className="welcome-controls">
            <button
              className={mode === "plan" ? "composer-control plan" : "composer-control build"}
              onClick={() => onModeChange(mode === "plan" ? "build" : "plan")}
            >
              {mode === "plan" ? "Plan" : "Build"}
            </button>
            <div className="composer-menu">
              <button
                className="composer-control neutral"
                onClick={() => {
                  setModelOpen((current) => !current);
                  setThinkingOpen(false);
                }}
              >
                <span>
                  {activeModel
                    ? `${activeModel.provider_display_name}/${activeModel.model_display_name}`
                    : "Select model"}
                </span>
                <ChevronDown size={13} />
              </button>
              {modelOpen ? (
                <div className="composer-popover model-popover">
                  {models.map((model) => (
                    <button
                      className={model.active ? "composer-option selected" : "composer-option"}
                      disabled={!model.connected}
                      key={`${model.provider_id}:${model.model_id}`}
                      onClick={() => {
                        onSelectModel(model);
                        setModelOpen(false);
                      }}
                    >
                      <span>
                        {model.provider_display_name}/{model.model_display_name}
                      </span>
                      <small>{model.connected ? "Connected" : "Not connected"}</small>
                    </button>
                  ))}
                </div>
              ) : null}
            </div>
            {activeModel?.thinking_levels.length ? (
              <div className="composer-menu">
                <button
                  className="composer-control thinking"
                  onClick={() => {
                    setThinkingOpen((current) => !current);
                    setModelOpen(false);
                  }}
                >
                  <span>{formatThinkingLevel(thinkingLevel ?? activeModel.thinking_level)}</span>
                  <ChevronDown size={13} />
                </button>
                {thinkingOpen ? (
                  <div className="composer-popover thinking-popover">
                    {activeModel.thinking_levels.map((level) => (
                      <button
                        className={
                          thinkingLevel === level ? "composer-option selected" : "composer-option"
                        }
                        key={level}
                        onClick={() => {
                          onSelectThinkingLevel(level);
                          setThinkingOpen(false);
                        }}
                      >
                        {formatThinkingLevel(level)}
                      </button>
                    ))}
                  </div>
                ) : null}
              </div>
            ) : null}
          </div>
          <button
            className="send-button"
            disabled={!draft.trim() || loading || sending}
            onClick={onSubmit}
            title="Start conversation"
          >
            {sending ? <LoaderCircle className="spin" size={17} /> : <Send size={17} />}
          </button>
        </div>
      </div>
      {error ? <div className="error-banner welcome-error">{error}</div> : null}
      {sessions.length > 0 ? (
        <div className="recent-sessions">
          <div className="recent-heading">
            <Clock3 size={16} />
            <span>Recent Sessions</span>
          </div>
          <div className="recent-session-grid">
            {sessions.slice(0, 5).map((session) => (
              <button
                className="recent-session"
                key={session.session_id}
                onClick={() => onSelectSession(session.session_id)}
              >
                <span>{session.title || "Untitled conversation"}</span>
                <time>{formatDate(session.updated_at)}</time>
              </button>
            ))}
          </div>
        </div>
      ) : null}
    </section>
  );
}

function VirtualMessageList({
  messages,
  streams,
}: {
  messages: MessageRecord[];
  streams: StreamMessage[];
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const rows = useMemo(
    () => [
      ...messages.map((record) => ({ type: "message" as const, key: record.message.id, record })),
      ...streams.map((stream) => ({ type: "stream" as const, key: stream.key, stream })),
    ],
    [messages, streams],
  );
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 130,
    overscan: 8,
    getItemKey: (index) => rows[index]?.key ?? index,
  });

  if (rows.length === 0) {
    return (
      <div className="welcome-state">
        <div className="welcome-icon">
          <Sparkles size={21} />
        </div>
        <h2>What are we building?</h2>
        <p>
          Start a conversation with the local tidev runtime. Your messages and streamed responses
          are persisted in SQLite.
        </p>
      </div>
    );
  }

  return (
    <div className="message-scroll" ref={scrollRef}>
      <div className="message-virtual-canvas" style={{ height: `${virtualizer.getTotalSize()}px` }}>
        {virtualizer.getVirtualItems().map((item) => {
          const row = rows[item.index];
          return (
            <div
              className="message-row"
              data-index={item.index}
              key={item.key}
              ref={virtualizer.measureElement}
              style={{ transform: `translateY(${item.start}px)` }}
            >
              {row.type === "message" ? (
                <MessageBubble record={row.record} />
              ) : (
                <StreamBubble stream={row.stream} />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function MessageBubble({ record }: { record: MessageRecord }) {
  const { message } = record;
  const isUser = message.role === "user";
  return (
    <article className={isUser ? "message-card user" : "message-card assistant"}>
      <div className="message-layout">
        <span className={isUser ? "avatar user-avatar" : "avatar"}>{isUser ? "U" : "A"}</span>
        <div className="message-column">
          <div className="message-meta">
            <span>{roleLabel(message.role)}</span>
            {message.created_at ? <time>{formatTime(message.created_at)}</time> : null}
          </div>
          <div className="message-content">
            {message.reasoning ? (
              <details className="reasoning">
                <summary>Reasoning</summary>
                <div>{message.reasoning}</div>
              </details>
            ) : null}
            {message.content ? (
              isUser ? (
                <p className="plain-content">{message.content}</p>
              ) : (
                <ReactMarkdown remarkPlugins={[remarkGfm]}>{message.content}</ReactMarkdown>
              )
            ) : (
              <span className="muted">No text content</span>
            )}
            {message.tool_calls?.length ? <ToolCallList calls={message.tool_calls} /> : null}
          </div>
        </div>
      </div>
    </article>
  );
}

function StreamBubble({ stream }: { stream: StreamMessage }) {
  return (
    <article className="message-card assistant streaming-card">
      <div className="message-layout">
        <span className="avatar">A</span>
        <div className="message-column">
          <div className="message-meta">
            <span>Assistant</span>
            <span className="streaming-label">streaming</span>
          </div>
          <div className="message-content">
            {stream.reasoning ? (
              <details className="reasoning" open>
                <summary>Reasoning</summary>
                <div>{stream.reasoning}</div>
              </details>
            ) : null}
            {stream.content ? (
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{stream.content}</ReactMarkdown>
            ) : (
              <span className="cursor-block" />
            )}
            {stream.toolCalls.length ? <ToolCallList calls={stream.toolCalls} /> : null}
            {stream.status === "failed" ? (
              <div className="stream-error">{stream.error ?? "The turn failed."}</div>
            ) : null}
          </div>
        </div>
      </div>
    </article>
  );
}

function ToolCallList({ calls }: { calls: ToolCall[] }) {
  return (
    <div className="tool-list">
      {calls.map((call) => (
        <div className="tool-chip" key={call.id}>
          <span>{call.name}</span>
          <code>{call.arguments}</code>
        </div>
      ))}
    </div>
  );
}

function ApprovalCard({
  request,
  onRespond,
}: {
  request: FrontendRequest;
  onRespond: (tools: ApprovedTool[]) => void;
}) {
  const tools = request.kind.ToolApproval ?? [];
  return (
    <div className="approval-card">
      <div className="approval-heading">
        <span>
          <Sparkles size={16} /> Approval required
        </span>
        <span className="approval-session">{request.session_id.slice(0, 8)}</span>
      </div>
      <p>
        tidev is waiting for permission to run{" "}
        {tools.length === 1 ? "a tool" : `${tools.length} tools`}.
      </p>
      <div className="approval-tools">
        {tools.map((item) => (
          <div className="approval-tool" key={item.tool_call.id}>
            <strong>{item.tool_call.name}</strong>
            <code>{item.tool_call.arguments}</code>
          </div>
        ))}
      </div>
      <div className="approval-actions">
        <button
          className="secondary-button"
          onClick={() => onRespond(tools.map((item) => makeRejectedTool(item.tool_call)))}
        >
          <X size={15} />
          Reject
        </button>
        <button
          className="primary-button"
          onClick={() => onRespond(tools.map((item) => makeApprovedTool(item.tool_call)))}
        >
          <Check size={15} />
          Allow
        </button>
      </div>
    </div>
  );
}

function DeferredFeature({ feature }: { feature: Feature }) {
  const data = {
    files: {
      icon: FolderTree,
      title: "Files",
      text: "File browsing and diffs are connected to the same runtime contract next.",
    },
    terminal: {
      icon: Terminal,
      title: "Terminal",
      text: "Interactive shell output will appear here without changing the chat transport.",
    },
    git: {
      icon: GitBranch,
      title: "Git",
      text: "Repository status, history, and changes will use the core Git service.",
    },
    stats: {
      icon: BarChart3,
      title: "Stats",
      text: "Usage and session statistics will be shown here.",
    },
    chat: { icon: MessageSquare, title: "Chat", text: "" },
  }[feature];
  const Icon = data.icon;
  return (
    <section className="deferred-feature">
      <div className="deferred-icon">
        <Icon size={25} />
      </div>
      <span className="eyebrow">Feature area</span>
      <h1>{data.title}</h1>
      <p>{data.text}</p>
    </section>
  );
}

function formatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" }).format(date);
}

function shortPath(value: string): string {
  if (!value) return "";
  const parts = value.split(/[\\/]/).filter(Boolean);
  return parts.length > 2 ? `…/${parts.slice(-2).join("/")}` : value;
}

function formatTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(date);
}

function formatThinkingLevel(value: string): string {
  const [, level = value] = value.split(":", 2);
  return level.charAt(0).toUpperCase() + level.slice(1);
}

function readEnterToSendPreference(): boolean {
  try {
    const settings = JSON.parse(localStorage.getItem("tidev-ui-settings") ?? "{}") as {
      enterToSend?: boolean;
    };
    return settings.enterToSend ?? true;
  } catch {
    return true;
  }
}
