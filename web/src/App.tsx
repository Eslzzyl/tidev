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
import { buildRounds } from "./utils/round";
import type { Round, ShellBlock, SystemMessageBlock } from "./utils/round";
import { formatTime as formatChatTime, getDuration, stripSystemReminderTags } from "./utils/format";
import { parseSlashCommand } from "./commands";
import { FileMentionPopover } from "./components/FileMentionPopover";
import { FilesView } from "./components/views/FilesView";
import { GitView } from "./components/views/GitView";
import { StatsView } from "./components/views/StatsView";
import { TerminalView } from "./components/views/TerminalView";

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
  const [fileMention, setFileMention] = useState<{ query: string; atPos: number } | null>(null);
  const [fileMentionIndex, setFileMentionIndex] = useState(0);
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

  const findAtFragment = useCallback((text: string, cursor: number): { atPos: number; query: string } | null => {
    const safeCursor = Math.min(cursor, text.length);
    const prefix = text.slice(0, safeCursor);
    const atIndex = prefix.lastIndexOf("@");
    if (atIndex === -1) return null;
    if (atIndex > 0) {
      const prev = prefix[atIndex - 1];
      if (prev && !/\s/.test(prev) && !["(", "[", "{", '"', "/", "\\"].includes(prev)) return null;
    }
    const query = prefix.slice(atIndex + 1);
    if (query.length > 0 && /\s/.test(query)) return null;
    return { atPos: atIndex, query };
  }, []);

  const updateFileMention = useCallback((text: string, cursor: number) => {
    const fragment = findAtFragment(text, cursor);
    if (fragment) {
      setFileMention(fragment);
      setFileMentionIndex(0);
    } else {
      setFileMention(null);
    }
  }, [findAtFragment]);

  const handleFileSelect = useCallback((path: string) => {
    if (!fileMention) return;
    const before = draft.slice(0, fileMention.atPos);
    const after = draft.slice(fileMention.atPos + 1 + fileMention.query.length);
    const inserted = `${before}@${path} ${after}`;
    setDraft(inserted);
    setFileMention(null);
    requestAnimationFrame(() => {
      const ta = composerTextareaRef.current;
      if (!ta) return;
      const newPos = before.length + 1 + path.length + 1;
      ta.focus();
      ta.setSelectionRange(newPos, newPos);
      updateFileMention(inserted, newPos);
    });
  }, [draft, fileMention, updateFileMention]);

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

  const handleRevert = useCallback(async (messageId: string) => {
    const sessionId = selectedSessionRef.current;
    if (!sessionId) return;
    try {
      await api.revert(sessionId, messageId);
      await loadMessages(sessionId);
      await loadTodos(sessionId);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Failed to revert");
    }
  }, [loadMessages, loadTodos]);

  const handleRedo = useCallback(async () => {
    const sessionId = selectedSessionRef.current;
    if (!sessionId) return;
    try {
      await api.redo(sessionId);
      await loadMessages(sessionId);
      await loadTodos(sessionId);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Failed to redo");
    }
  }, [loadMessages, loadTodos]);

  const handleFork = useCallback(async (messageId: string) => {
    const sessionId = selectedSessionRef.current;
    if (!sessionId) return;
    try {
      const forked = await api.fork(sessionId, messageId);
      setSessions((current) => [forked, ...current]);
      selectSession(forked.session_id);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Failed to fork");
    }
  }, [selectSession]);

  const handleCompact = useCallback(async () => {
    const sessionId = selectedSessionRef.current;
    if (!sessionId) return;
    try {
      await api.compact(sessionId);
      setError(null);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Failed to compact");
    }
  }, []);

  const handleShell = useCallback(async (command: string) => {
    const sessionId = selectedSessionRef.current;
    if (!sessionId || !command.trim()) return;
    try {
      await api.shell(sessionId, command);
      // Shell output will appear via subsequent message reload triggered by
      // the shell task appending messages; poll briefly.
      setTimeout(() => void loadMessages(sessionId), 400);
      setTimeout(() => void loadMessages(sessionId), 1200);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Failed to run shell command");
    }
  }, [loadMessages]);

  const handleSlashCommand = useCallback(async (raw: string): Promise<boolean> => {
    const parsed = parseSlashCommand(raw);
    if (!parsed) return false;
    const { command, args } = parsed;
    if (command === "undo") {
      const lastUser = [...messages].reverse().find((r) => r.message.role === "user");
      if (lastUser) await handleRevert(lastUser.message.id);
      else setError("No user message to undo");
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
      else setError("No message to fork from");
      return true;
    }
    if (command === "shell") {
      if (!args) {
        setError("Usage: /shell <command> or !<command>");
        return true;
      }
      await handleShell(args);
      return true;
    }
    if (command === "rename" && args) {
      const sid = selectedSessionRef.current;
      if (sid) {
        try {
          const updated = await api.updateSession(sid, args);
          setSessions((current) => current.map((item) => (item.session_id === sid ? updated : item)));
        } catch (reason) {
          setError(reason instanceof Error ? reason.message : "Failed to rename");
        }
      }
      return true;
    }
    if (command === "new") {
      createSession();
      return true;
    }
    return false;
  }, [messages, handleRevert, handleRedo, handleFork, handleCompact, handleShell]);

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
                    <button className="ghost-button" onClick={() => void handleRedo()} title="Redo">
                      Redo
                    </button>
                    <button className="ghost-button" onClick={() => void handleCompact()} title="Compact context">
                      Compact
                    </button>
                    <span className="model-label">
                      <Sparkles size={15} />
                      {selectedSession?.model_display_name ?? "Runtime model"}
                    </span>
                  </div>
                </div>
                <div className="message-stage">
                  <VirtualMessageList
                    messages={messages}
                    streams={visibleStreams}
                    onRevert={(id) => void handleRevert(id)}
                    onFork={(id) => void handleFork(id)}
                  />
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
                  <div className="composer" style={{ position: "relative" }}>
                    {fileMention ? (
                      <FileMentionPopover
                        query={fileMention.query}
                        selectedIndex={fileMentionIndex}
                        onSelectedIndexChange={setFileMentionIndex}
                        onSelect={handleFileSelect}
                        onClose={() => setFileMention(null)}
                      />
                    ) : null}
                    <textarea
                      ref={composerTextareaRef}
                      value={draft}
                      onChange={(event) => {
                        const value = event.target.value;
                        const cursor = event.target.selectionStart ?? value.length;
                        setDraft(value);
                        updateFileMention(value, cursor);
                      }}
                      onSelect={(event) => {
                        const ta = event.target as HTMLTextAreaElement;
                        updateFileMention(ta.value, ta.selectionStart ?? ta.value.length);
                      }}
                      onClick={(event) => {
                        const ta = event.target as HTMLTextAreaElement;
                        updateFileMention(ta.value, ta.selectionStart ?? ta.value.length);
                      }}
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
                        if (fileMention) {
                          if (event.key === "Escape") {
                            event.preventDefault();
                            setFileMention(null);
                            return;
                          }
                          if (event.key === "Enter" || event.key === "Tab") {
                            // Let FileMentionPopover handle selection via its document listener
                            // but prevent the composer from submitting.
                            event.preventDefault();
                            return;
                          }
                          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                            event.preventDefault();
                            return;
                          }
                        }
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
        ) : feature === "files" ? (
          <FilesView />
        ) : feature === "terminal" ? (
          <TerminalView />
        ) : feature === "git" ? (
          <GitView />
        ) : feature === "stats" ? (
          <StatsView />
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
  onRevert,
  onFork,
}: {
  messages: MessageRecord[];
  streams: StreamMessage[];
  onRevert?: (messageId: string) => void;
  onFork?: (messageId: string) => void;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const rounds = useMemo(() => buildRounds(messages), [messages]);
  type RoundRow = { type: "round"; key: string; round: Round };
  type SystemRow = { type: "system"; key: string; block: SystemMessageBlock };
  type ShellRow = { type: "shell"; key: string; block: ShellBlock };
  type StreamRow = { type: "stream"; key: string; stream: StreamMessage };
  type Row = RoundRow | SystemRow | ShellRow | StreamRow;
  const rows = useMemo<Row[]>(() => {
    const base: Row[] = rounds.map((item) => {
      if ((item as ShellBlock).kind === "shell") {
        const b = item as ShellBlock;
        return { type: "shell", key: b.id, block: b } as ShellRow;
      }
      if ((item as SystemMessageBlock).kind === "system") {
        const b = item as SystemMessageBlock;
        return { type: "system", key: b.id, block: b } as SystemRow;
      }
      const r = item as Round;
      return { type: "round", key: r.id, round: r } as RoundRow;
    });
    for (const s of streams) {
      base.push({ type: "stream", key: s.key, stream: s } as StreamRow);
    }
    return base;
  }, [rounds, streams]);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 160,
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
              {row.type === "round" ? (
                <RoundView round={row.round} onRevert={onRevert} onFork={onFork} />
              ) : row.type === "shell" ? (
                <ShellBlockView block={row.block} />
              ) : row.type === "system" ? (
                <SystemBlockView block={row.block} />
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

function RoundView({
  round,
  onRevert,
  onFork,
}: {
  round: Round;
  onRevert?: (messageId: string) => void;
  onFork?: (messageId: string) => void;
}) {
  const userTime = round.userMessage.created_at ? formatChatTime(round.userMessage.created_at) : "";
  const duration = round.completedAt
    ? getDuration(round.userMessage.created_at ?? "", round.completedAt)
    : null;
  const footerParts: string[] = [];
  if (round.modelName) footerParts.push(round.modelName);
  if (duration) footerParts.push(duration);
  if (round.completedAt) footerParts.push(formatChatTime(round.completedAt));
  else if (round.status === "streaming" && userTime) footerParts.push(userTime);

  const hasAssistant = round.segments.length > 0 || round.status !== "user_only";
  return (
    <div className="round-group">
      <article className="message-card user">
        <div className="message-layout">
          <span className="avatar user-avatar">U</span>
          <div className="message-column">
            <div className="message-meta">
              <span>You</span>
              {userTime ? <time>{userTime}</time> : null}
              <span className="message-actions-inline">
                {onRevert ? (
                  <button
                    className="inline-action"
                    onClick={() => onRevert(round.userMessage.id)}
                    title="Revert to this message (undo later messages)"
                  >
                    Undo
                  </button>
                ) : null}
                {onFork ? (
                  <button
                    className="inline-action"
                    onClick={() => onFork(round.userMessage.id)}
                    title="Fork conversation from this message"
                  >
                    Fork
                  </button>
                ) : null}
              </span>
            </div>
            <div className="message-content">
              <p className="plain-content">{stripSystemReminderTags(round.userMessage.content)}</p>
            </div>
          </div>
        </div>
      </article>
      {hasAssistant ? (
        <article className="message-card assistant">
          <div className="message-layout">
            <span className="avatar">A</span>
            <div className="message-column">
              <div className="message-meta">
                <span>Assistant</span>
                {round.status === "streaming" ? (
                  <span className="streaming-label">streaming</span>
                ) : null}
              </div>
              <div className="message-content">
                {round.segments.map((seg, idx) => {
                  if (seg.type === "reasoning" && seg.content) {
                    return (
                      <details key={idx} className="reasoning" open={round.status === "streaming"}>
                        <summary>Reasoning</summary>
                        <div>{seg.content}</div>
                      </details>
                    );
                  }
                  if (seg.type === "text" && seg.content) {
                    return (
                      <ReactMarkdown key={idx} remarkPlugins={[remarkGfm]}>
                        {stripSystemReminderTags(seg.content)}
                      </ReactMarkdown>
                    );
                  }
                  if (seg.type === "tool_call") {
                    const entry = round.toolCallMap[seg.toolCallId];
                    if (!entry) return null;
                    return <ToolCallEntryView key={idx} entry={entry} />;
                  }
                  return null;
                })}
                {round.status === "streaming" && round.segments.length === 0 ? (
                  <span className="cursor-block" />
                ) : null}
                {round.status === "complete" && footerParts.length ? (
                  <div className="round-footer">{footerParts.join(" · ")}</div>
                ) : null}
              </div>
            </div>
          </div>
        </article>
      ) : null}
    </div>
  );
}

function SystemBlockView({ block }: { block: SystemMessageBlock }) {
  return (
    <article className="message-card system">
      <div className="message-layout">
        <span className="avatar">S</span>
        <div className="message-column">
          <div className="message-meta">
            <span>System</span>
            {block.message.created_at ? <time>{formatChatTime(block.message.created_at)}</time> : null}
          </div>
          <div className="message-content">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{block.message.content}</ReactMarkdown>
          </div>
        </div>
      </div>
    </article>
  );
}

function ShellBlockView({ block }: { block: ShellBlock }) {
  const exit = block.exitCode;
  return (
    <article className="message-card shell">
      <div className="message-layout">
        <span className="avatar">S</span>
        <div className="message-column">
          <div className="message-meta">
            <span>Shell</span>
            {exit !== null && exit !== undefined ? (
              <span className={exit === 0 ? "shell-exit ok" : "shell-exit fail"}>exit {exit}</span>
            ) : null}
          </div>
          <div className="message-content">
            <pre className="shell-command">{block.command.content}</pre>
            <pre className="shell-output">{block.output.content}</pre>
          </div>
        </div>
      </div>
    </article>
  );
}

function ToolCallEntryView({ entry }: { entry: import("./utils/round").ToolCallEntry }) {
  const summary = (() => {
    try {
      const args = JSON.parse(entry.arguments);
      if (entry.name === "read" || entry.name === "write" || entry.name === "edit") {
        return args.file_path || args.path || "";
      }
      if (entry.name === "bash") return args.command || "";
      if (entry.name === "grep") return args.pattern ? `"${args.pattern}"` : "";
      if (entry.name === "glob") return args.pattern || "";
      return entry.arguments.slice(0, 80);
    } catch {
      return entry.arguments.slice(0, 80);
    }
  })();
  return (
    <div className="tool-entry">
      <div className="tool-entry-header">
        <strong>{entry.name}</strong>
        <code className="tool-args">{summary}</code>
        {!entry.resultComplete ? <span className="tool-running">running…</span> : null}
      </div>
      {entry.result ? (
        <pre className="tool-result">{entry.result.output.slice(0, 2000)}</pre>
      ) : null}
    </div>
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
