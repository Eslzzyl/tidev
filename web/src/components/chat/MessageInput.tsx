import { useState, useEffect, useRef, useCallback } from "react";
import {
  ChevronDown,
  Square,
  ArrowUp,
  ListTodo,
  CheckCircle2,
  Circle,
  Loader2,
} from "lucide-react";
import { useSessionStore } from "../../stores/useSessionStore";
import { useUIStore } from "../../stores/useUIStore";
import { FileMentionPopover } from "./FileMentionPopover";
import { commandFragment, getSuggestions } from "../../commands";
import { api } from "../../api/client";
import type { ModelInfo, FileSuggestion, TodoItem } from "../../types/api";
import type { CommandSuggestion } from "../../commands";
import { ModelPanel } from "./ModelPanel";

interface MessageInputProps {
  onSlashCommand?: (command: string) => void;
  skillInsert?: { text: string } | null;
}

export function MessageInput({
  onSlashCommand,
  skillInsert,
}: MessageInputProps) {
  const [inputValue, setInputValue] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // File mention (@) state
  const [fileMention, setFileMention] = useState<{
    visible: boolean;
    query: string;
    atPosition: number;
    cursorPosition: { x: number; y: number };
  } | null>(null);

  // Command palette (/command) state
  const [commandPalette, setCommandPalette] = useState<{
    visible: boolean;
    selectedIndex: number;
    suggestions: CommandSuggestion[];
    position: { x: number; y: number };
  }>({
    visible: false,
    selectedIndex: 0,
    suggestions: [],
    position: { x: 0, y: 0 },
  });

  // Models state
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [modelPanelOpen, setModelPanelOpen] = useState(false);
  const [selectedModelId, setSelectedModelId] = useState<string | null>(null);
  const [selectedProviderId, setSelectedProviderId] = useState<string | null>(
    null,
  );
  type ThinkingOption = { label: string; value: string };
  const [thinkingOptions, setThinkingOptions] = useState<ThinkingOption[]>([]);
  const [selectedThinking, setSelectedThinking] = useState<string>("");
  const [thinkingDropdownOpen, setThinkingDropdownOpen] = useState(false);

  const thinkingDropdownRef = useRef<HTMLDivElement>(null);
  const todoDropdownRef = useRef<HTMLDivElement>(null);

  // Todos state
  const [todos, setTodos] = useState<TodoItem[]>([]);
  const [todoDropdownOpen, setTodoDropdownOpen] = useState(false);
  const [isLoadingTodos, setIsLoadingTodos] = useState(false);

  const currentSessionId = useSessionStore((s) => s.currentSessionId);
  const currentSession = useSessionStore((s) => s.currentSession);
  const isDraftSession = useSessionStore((s) => s.isDraftSession);
  const mode = useSessionStore((s) => s.mode);
  const toggleMode = useSessionStore((s) => s.toggleMode);
  const commitDraftSession = useSessionStore((s) => s.commitDraftSession);
  const setCurrentSessionId = useSessionStore((s) => s.setCurrentSessionId);
  const setCurrentRequestId = useSessionStore((s) => s.setCurrentRequestId);
  const currentRequestId = useSessionStore((s) => s.currentRequestId);
  const setMessages = useSessionStore((s) => s.setMessages);
  const isStreaming = useUIStore((s) => s.isStreaming);
  const setStreaming = useUIStore((s) => s.setStreaming);
  const setError = useSessionStore((s) => s.setError);

  const isInputEnabled = currentSessionId !== null || isDraftSession;

  // Update thinking levels based on model - use data from API model info
  const updateThinkingLevels = useCallback((modelId: string) => {
    const model = models.find((m) => m.id === modelId);
    if (model && model.thinking_supported && model.thinking_options.length > 0) {
      const options = model.thinking_options.map((opt) => {
        const parts = opt.split(":");
        const label = parts[1] ? parts[1].charAt(0).toUpperCase() + parts[1].slice(1) : opt;
        return { label, value: opt };
      });
      setThinkingOptions(options);
      // Prefer the model's default thinking level, fall back to first option
      const defaultTl = model.thinking_options.includes(model.thinking_level)
        ? model.thinking_level
        : model.thinking_options[0];
      setSelectedThinking(defaultTl);
    } else {
      setThinkingOptions([]);
      setSelectedThinking("");
    }
  }, [models]);

  // Load models and set initial selection
  useEffect(() => {
    api
      .listModels()
      .then(({ models: modelList }) => {
        setModels(modelList);

        // Prefer the session's current model, otherwise default to first model
        const sessionModelId = currentSession?.model_id;
        const sessionProviderId = currentSession?.provider_id;
        const sessionModel =
          sessionModelId && sessionProviderId
            ? modelList.find(
                (m) =>
                  m.id === sessionModelId &&
                  m.provider_id === sessionProviderId,
              )
            : null;

        if (sessionModel) {
          setSelectedModelId(sessionModel.id);
          setSelectedProviderId(sessionModel.provider_id);
          updateThinkingLevels(sessionModel.id);
        } else if (!selectedModelId && modelList.length > 0) {
          setSelectedModelId(modelList[0].id);
          setSelectedProviderId(modelList[0].provider_id);
          updateThinkingLevels(modelList[0].id);
        }
      })
      .catch(() => {});
  }, [selectedModelId, updateThinkingLevels, currentSession]);

  // Close dropdowns on click outside
  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (
        thinkingDropdownRef.current &&
        !thinkingDropdownRef.current.contains(e.target as Node)
      ) {
        setThinkingDropdownOpen(false);
      }
      if (
        todoDropdownRef.current &&
        !todoDropdownRef.current.contains(e.target as Node)
      ) {
        setTodoDropdownOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  // Load todos when session changes
  useEffect(() => {
    if (!currentSessionId) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setTodos([]);
      return;
    }

    setIsLoadingTodos(true);
    api
      .getTodos(currentSessionId)
      .then((response) => {
        setTodos(response.todos);
      })
      .catch(() => {
        setTodos([]);
      })
      .finally(() => {
        setIsLoadingTodos(false);
      });
  }, [currentSessionId]);

  // Auto-resize textarea
  useEffect(() => {
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
      textareaRef.current.style.height =
        Math.min(textareaRef.current.scrollHeight, 200) + "px";
    }
  }, [inputValue]);

  // Handle skill insert
  useEffect(() => {
    if (skillInsert?.text && textareaRef.current) {
      const cursorPos = textareaRef.current.selectionStart || 0;
      const newValue =
        inputValue.slice(0, cursorPos) +
        skillInsert.text +
        inputValue.slice(cursorPos);
      setInputValue(newValue);
      textareaRef.current.focus();
    }
  }, [skillInsert, inputValue]);

  // IME composition guards.
  //
  // On macOS with Chinese Pinyin IME, pressing Enter to commit fires
  // `compositionend` BEFORE `keydown` in the same synchronous dispatch.
  // By the time keydown fires, both `isComposing` and a simple useRef
  // are already false, so the Enter key incorrectly submits the message.
  //
  // Fix: set a "just committed" flag on compositionend and clear it in a
  // microtask (Promise.resolve). Since compositionend and keydown fire in
  // the same synchronous dispatch, the flag is still true when keydown runs,
  // preventing premature submission.
  const composingRef = useRef(false);
  const compositionJustCommittedRef = useRef(false);

  function handleCompositionStart() {
    composingRef.current = true;
    compositionJustCommittedRef.current = false;
  }

  function handleCompositionEnd(_e: React.CompositionEvent<HTMLTextAreaElement>) {
    composingRef.current = false;
    if (_e.data) {
      compositionJustCommittedRef.current = true;
      void Promise.resolve().then(() => {
        compositionJustCommittedRef.current = false;
      });
    }
  }

  function handleKeydown(event: React.KeyboardEvent) {
    // Command palette navigation takes priority
    if (commandPalette.visible && commandPalette.suggestions.length > 0) {
      if (event.key === "Escape") {
        event.preventDefault();
        closeCommandPalette();
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setCommandPalette((prev) => ({
          ...prev,
          selectedIndex:
            prev.selectedIndex > 0
              ? prev.selectedIndex - 1
              : prev.suggestions.length - 1,
        }));
        return;
      }
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setCommandPalette((prev) => ({
          ...prev,
          selectedIndex:
            prev.selectedIndex < prev.suggestions.length - 1
              ? prev.selectedIndex + 1
              : 0,
        }));
        return;
      }
      if (event.key === "Tab" || event.key === "Enter") {
        event.preventDefault();
        const selected =
          commandPalette.suggestions[commandPalette.selectedIndex];
        if (selected) {
          executeCommand(selected.spec.name);
        }
        return;
      }
    }

    // Don't handle Tab/Enter if file mention popover is visible
    if (fileMention?.visible) {
      if (event.key === "Enter" || event.key === "Tab") {
        return;
      }
    }

    if (event.key === "Tab") {
      event.preventDefault();
      toggleMode();
      return;
    }
    if (
      event.key === "Enter" &&
      !event.shiftKey &&
      !composingRef.current &&
      !compositionJustCommittedRef.current
    ) {
      event.preventDefault();
      handleSubmit();
    }
  }

  async function handleSubmit() {
    const content = inputValue.trim();
    if (!content || !isInputEnabled || isSubmitting) return;

    // Check for slash commands
    if (content.startsWith("/")) {
      handleSlashCommand(content);
      return;
    }

    setIsSubmitting(true);
    setStreaming(true);

    try {
      let sessionId = currentSessionId;

      // If draft session, create one first
      if (!sessionId) {
        const workspace = await api.getWorkspace();
        const { session_id } = await api.createSession({
          workspace_root: workspace.workspace_root,
          title: content.slice(0, 50),
          provider_id: selectedProviderId ?? undefined,
          model_id: selectedModelId ?? undefined,
        });
        sessionId = session_id;

        // Update store
        const [session, { messages, todos }] = await Promise.all([
          api.getSession(sessionId),
          api.listMessages(sessionId),
        ]);
        commitDraftSession(session);
        setMessages(messages);
        useSessionStore.getState().setTodos(todos ?? []);
        setCurrentSessionId(sessionId);

        // Update URL
        const url = new URL(window.location.href);
        url.searchParams.set("session", sessionId);
        window.history.replaceState({}, "", url.toString());
      }

      // Add the user message to the store immediately so the SSE handler
      // finds it as the last user message when creating the streaming round.
      const pendingId = `pending-${Date.now()}`;
      useSessionStore.getState().addMessage({
        id: pendingId,
        role: "user",
        content,
        created_at: new Date().toISOString(),
      });

      // Send message
      const requestBody: {
        content: string;
        mode?: string;
        model_id?: string;
        provider_id?: string;
        thinking_level?: string;
      } = { content };

      if (mode) requestBody.mode = mode;
      if (selectedModelId) requestBody.model_id = selectedModelId;
      if (selectedProviderId) requestBody.provider_id = selectedProviderId;
      if (selectedThinking) requestBody.thinking_level = selectedThinking;

      const { request_id } = await api.sendMessage(sessionId, requestBody);
      setCurrentRequestId(request_id);
      setInputValue("");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to send message");
      setStreaming(false);
    } finally {
      setIsSubmitting(false);
    }
  }

  function handleSlashCommand(command: string) {
    const cmd = command.toLowerCase();
    const name = cmd.startsWith("/") ? cmd.slice(1).split(" ")[0] : cmd;

    setInputValue("");

    if (name === "message" || name === "msg") {
      onSlashCommand?.("message");
    } else if (name === "undo") {
      executeUndo();
    } else if (name === "redo") {
      executeRedo();
    } else if (name === "compact") {
      executeCompact();
    } else if (name === "init") {
      executeInit();
    } else if (name === "rename" || name === "title") {
      onSlashCommand?.("rename");
    } else if (name === "new" || name === "clear") {
      useSessionStore.getState().goToWelcome();
    } else if (name === "skills" || name === "skill") {
      onSlashCommand?.("skills");
    }
  }

  async function executeCommand(name: string) {
    setInputValue("");
    closeCommandPalette();

    if (name === "message" || name === "msg") {
      onSlashCommand?.("message");
    } else if (name === "undo") {
      await executeUndo();
    } else if (name === "redo") {
      await executeRedo();
    } else if (name === "compact") {
      await executeCompact();
    } else if (name === "init") {
      await executeInit();
    } else if (name === "rename" || name === "title") {
      onSlashCommand?.("rename");
    } else if (name === "new" || name === "clear") {
      useSessionStore.getState().goToWelcome();
    } else if (name === "skills" || name === "skill") {
      onSlashCommand?.("skills");
    }
  }

  async function executeUndo() {
    const sessionId = currentSessionId;
    if (!sessionId) return;

    const messages = useSessionStore.getState().messages;
    const userMessages = messages.filter(
      (m) => m.role === "user" && !m.id.startsWith("pending-"),
    );
    if (userMessages.length === 0) return;

    const lastUserMessage = userMessages[userMessages.length - 1];

    try {
      await api.revertToMessage(sessionId, lastUserMessage.id);
      const { messages: updatedMessages, todos } =
        await api.listMessages(sessionId);
      useSessionStore.getState().setMessages(updatedMessages);
      useSessionStore.getState().setTodos(todos ?? []);
    } catch (error) {
      console.error("Undo failed:", error);
    }
  }

  async function executeRedo() {
    const sessionId = currentSessionId;
    if (!sessionId) return;

    try {
      await api.redoSession(sessionId);
      const { messages: updatedMessages, todos } =
        await api.listMessages(sessionId);
      useSessionStore.getState().setMessages(updatedMessages);
      useSessionStore.getState().setTodos(todos ?? []);
    } catch (error) {
      console.error("Redo failed:", error);
    }
  }

  async function executeInit() {
    try {
      const { prompt } = await api.getInitPrompt();
      setInputValue(prompt);
      setTimeout(() => textareaRef.current?.focus(), 0);
    } catch (error) {
      console.error("Failed to load init prompt:", error);
    }
  }

  async function executeCompact() {
    const sessionId = currentSessionId;
    if (!sessionId) return;

    try {
      await api.compactSession(sessionId);
      // The compaction runs in the background; SSE will send
      // messages.updated when it completes, triggering a refresh.
    } catch (error) {
      console.error("Compact failed:", error);
    }
  }

  function closeCommandPalette() {
    setCommandPalette({
      visible: false,
      selectedIndex: 0,
      suggestions: [],
      position: { x: 0, y: 0 },
    });
  }

  async function handleStop() {
    if (currentSessionId && currentRequestId) {
      try {
        await api.abortRequest(currentSessionId, {
          request_id: currentRequestId,
        });
      } catch {
        // ignore
      }
      setStreaming(false);
      setCurrentRequestId(null);
    }
  }

  // Detect @ mention in input
  function detectAtFragment(
    input: string,
    cursor: number,
  ): { atIndex: number; query: string } | null {
    const prefix = input.slice(0, cursor);
    const atIndex = prefix.lastIndexOf("@");
    if (atIndex === -1) return null;

    // Check if @ is preceded by valid character
    if (atIndex > 0) {
      const prev = prefix[atIndex - 1];
      if (!/\s/.test(prev) && !/[([{"'/\\]/.test(prev)) {
        return null;
      }
    }

    const query = prefix.slice(atIndex + 1);
    // Query cannot contain whitespace
    if (/\s/.test(query)) return null;

    return { atIndex, query };
  }

  // Calculate popover position based on textarea and cursor
  function calculatePopoverPosition(
    textarea: HTMLTextAreaElement,
    atIndex: number,
  ): { x: number; y: number } {
    const textBeforeAt = textarea.value.slice(0, atIndex);
    const lines = textBeforeAt.split("\n");
    const currentLineText = lines[lines.length - 1];

    // Create a mirror element to measure text position
    const mirror = document.createElement("div");
    const computedStyle = getComputedStyle(textarea);
    mirror.style.cssText = `
      position: fixed;
      top: 0;
      left: 0;
      visibility: hidden;
      white-space: pre-wrap;
      word-wrap: break-word;
      font: ${computedStyle.font};
      padding: ${computedStyle.padding};
      border: ${computedStyle.border};
      width: ${textarea.clientWidth}px;
      line-height: ${computedStyle.lineHeight};
    `;
    mirror.textContent = currentLineText;
    document.body.appendChild(mirror);

    // Measure the width of text before @
    const textSpan = document.createElement("span");
    textSpan.textContent = currentLineText;
    mirror.appendChild(textSpan);

    const textRect = textSpan.getBoundingClientRect();
    const textareaRect = textarea.getBoundingClientRect();

    document.body.removeChild(mirror);

    // Calculate position: at the @ character, in viewport coordinates
    const x =
      textareaRect.left +
      textRect.width +
      parseInt(computedStyle.paddingLeft || "0");
    // Position at the top of current line (popover will extend upward)
    const y = textareaRect.top + parseInt(computedStyle.paddingTop || "0");

    return { x, y };
  }

  // Handle input change with @ detection and /command detection
  function handleInputChange(e: React.ChangeEvent<HTMLTextAreaElement>) {
    const value = e.target.value;
    const cursor = e.target.selectionStart || 0;

    setInputValue(value);

    // Check for @ file mention
    const atFragment = detectAtFragment(value, cursor);
    if (atFragment) {
      setFileMention({
        visible: true,
        query: atFragment.query,
        atPosition: atFragment.atIndex,
        cursorPosition: calculatePopoverPosition(e.target, atFragment.atIndex),
      });
    } else {
      setFileMention(null);
    }

    // Check for / command palette
    const fragment = commandFragment(value);
    if (fragment !== null) {
      const suggestions = getSuggestions(fragment);
      // Calculate position at the start of input (where / is)
      const position = calculatePopoverPosition(e.target, 0);
      setCommandPalette({
        visible: true,
        selectedIndex: 0,
        suggestions,
        position: { x: position.x, y: position.y },
      });
    } else {
      closeCommandPalette();
    }
  }

  // Handle file selection from popover
  function handleFileSelect(path: string, kind: FileSuggestion["kind"]) {
    if (!fileMention || !textareaRef.current) return;

    const before = inputValue.slice(0, fileMention.atPosition);
    const after = inputValue.slice(textareaRef.current.selectionStart);
    const replacement = kind === "directory" ? `@${path}/` : `@${path}`;

    const newValue = before + replacement + after;
    setInputValue(newValue);
    setFileMention(null);

    // Focus back to textarea and set cursor position
    setTimeout(() => {
      if (textareaRef.current) {
        textareaRef.current.focus();
        const newCursorPos = before.length + replacement.length;
        textareaRef.current.setSelectionRange(newCursorPos, newCursorPos);
      }
    }, 0);
  }

  // Called when the ModelPanel's General tab selects a new model
  function handleModelPanelChange(model: ModelInfo) {
    setSelectedModelId(model.id);
    setSelectedProviderId(model.provider_id);
    updateThinkingLevels(model.id);
  }

  const selectedModelDisplay = selectedModelId
    ? models.find((m) => m.id === selectedModelId)
    : null;

  return (
    <div className="border-t border-neutral-200 bg-white px-4 py-3 dark:border-neutral-800 dark:bg-neutral-950">
      <div className="mx-auto flex max-w-4xl flex-col gap-2">
        {/* Controls row */}
        <div className="flex items-center gap-2">
          {/* Mode toggle */}
          <button
            onClick={toggleMode}
            className={`rounded px-2 py-1 text-xs font-medium transition-colors ${
              mode === "plan"
                ? "bg-purple-100 text-purple-700 hover:bg-purple-200 dark:bg-purple-900/30 dark:text-purple-300"
                : "bg-emerald-100 text-emerald-700 hover:bg-emerald-200 dark:bg-emerald-900/30 dark:text-emerald-300"
            }`}
          >
            {mode === "plan" ? "Plan" : "Build"}
          </button>

          {/* Model selector - opens ModelPanel */}
          <button
            onClick={() => setModelPanelOpen(true)}
            className="flex items-center gap-1 rounded bg-neutral-100 px-2 py-1 text-xs text-neutral-700 hover:bg-neutral-200 dark:bg-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-700"
          >
            <span className="max-w-[120px] truncate">
              {selectedModelDisplay?.display_name || "Select model"}
            </span>
            <ChevronDown className="h-3 w-3" />
          </button>

          {/* Model Panel */}
          <ModelPanel
            isOpen={modelPanelOpen}
            onClose={() => setModelPanelOpen(false)}
            currentModelId={selectedModelId}
            currentProviderId={selectedProviderId}
            onModelChange={handleModelPanelChange}
          />

          {/* Thinking level selector */}
          {thinkingOptions.length > 0 && (
            <div ref={thinkingDropdownRef} className="relative">
              <button
                onClick={() => setThinkingDropdownOpen(!thinkingDropdownOpen)}
                className="flex items-center gap-1 rounded bg-amber-50 px-2 py-1 text-xs text-amber-700 hover:bg-amber-100 dark:bg-amber-950/30 dark:text-amber-300 dark:hover:bg-amber-900/50"
              >
                <span>
                  {thinkingOptions.find((t) => t.value === selectedThinking)
                    ?.label || "Thinking"}
                </span>
                <ChevronDown className="h-3 w-3" />
              </button>

              {thinkingDropdownOpen && (
                <div className="absolute bottom-full left-0 z-50 mb-1 w-36 rounded-lg border border-neutral-200 bg-white shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
                  {thinkingOptions.map((option) => (
                    <button
                      key={option.value}
                      onClick={() => {
                        setSelectedThinking(option.value);
                        setThinkingDropdownOpen(false);
                      }}
                      className={`flex w-full px-3 py-2 text-left text-xs hover:bg-neutral-100 dark:hover:bg-neutral-800 ${selectedThinking === option.value ? "bg-neutral-100 dark:bg-neutral-800 font-medium" : ""}`}
                    >
                      {option.label}
                    </button>
                  ))}
                </div>
              )}
            </div>
          )}

          {/* Spacer to push todo button to the right */}
          <div className="flex-1" />

          {/* Todo list dropdown */}
          {currentSessionId && todos.length > 0 && (
            <div ref={todoDropdownRef} className="relative">
              <button
                onClick={() => setTodoDropdownOpen(!todoDropdownOpen)}
                className="flex items-center gap-1.5 rounded bg-blue-50 px-2 py-1 text-xs text-blue-700 hover:bg-blue-100 dark:bg-blue-950/30 dark:text-blue-300 dark:hover:bg-blue-900/50"
              >
                <ListTodo className="h-3.5 w-3.5" />
                <span>
                  {todos.filter((t) => t.status === "completed").length}/
                  {todos.length}
                </span>
                <ChevronDown className="h-3 w-3" />
              </button>

              {todoDropdownOpen && (
                <div className="absolute bottom-full right-0 z-50 mb-1 w-64 rounded-lg border border-neutral-200 bg-white shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
                  <div className="border-b border-neutral-100 px-3 py-2 dark:border-neutral-800">
                    <span className="text-xs font-medium text-neutral-700 dark:text-neutral-300">
                      Todo List
                    </span>
                  </div>
                  <div className="max-h-48 overflow-y-auto py-1">
                    {isLoadingTodos ? (
                      <div className="flex items-center justify-center py-4">
                        <Loader2 className="h-4 w-4 animate-spin text-neutral-400" />
                      </div>
                    ) : todos.length === 0 ? (
                      <div className="px-3 py-2 text-xs text-neutral-500 dark:text-neutral-400">
                        No todos yet
                      </div>
                    ) : (
                      todos.map((todo, index) => (
                        <div
                          key={index}
                          className="flex items-start gap-2 px-3 py-1.5"
                        >
                          {todo.status === "completed" ? (
                            <CheckCircle2 className="mt-0.5 h-3.5 w-3.5 flex-shrink-0 text-green-500" />
                          ) : todo.status === "in_progress" ? (
                            <Loader2 className="mt-0.5 h-3.5 w-3.5 flex-shrink-0 animate-spin text-blue-500" />
                          ) : (
                            <Circle className="mt-0.5 h-3.5 w-3.5 flex-shrink-0 text-neutral-400" />
                          )}
                          <span
                            className={`text-xs ${
                              todo.status === "completed"
                                ? "text-neutral-500 line-through dark:text-neutral-500"
                                : "text-neutral-700 dark:text-neutral-300"
                            }`}
                          >
                            {todo.content}
                          </span>
                        </div>
                      ))
                    )}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>

        {/* Input row */}
        <div className="relative">
          {/* Command palette */}
          {commandPalette.visible && commandPalette.suggestions.length > 0 && (
            <div
              className="fixed z-50 w-full max-w-sm"
              style={{
                left: commandPalette.position.x,
                top: commandPalette.position.y - 8,
                transform: "translateY(-100%)",
              }}
            >
              <div className="overflow-hidden rounded-lg border border-neutral-200 bg-white shadow-xl dark:border-neutral-700 dark:bg-neutral-900">
                <div className="max-h-64 overflow-y-auto py-1">
                  {commandPalette.suggestions.map((suggestion, index) => (
                    <button
                      key={suggestion.spec.name}
                      onClick={() => {
                        executeCommand(suggestion.spec.name);
                        textareaRef.current?.focus();
                      }}
                      className={`flex w-full items-center gap-3 px-4 py-2 text-left ${
                        index === commandPalette.selectedIndex
                          ? "bg-blue-50 dark:bg-blue-900/30"
                          : "hover:bg-neutral-50 dark:hover:bg-neutral-800"
                      }`}
                    >
                      <span
                        className={`flex-shrink-0 rounded px-1.5 py-0.5 font-mono text-xs font-medium ${
                          index === commandPalette.selectedIndex
                            ? "bg-blue-200 text-blue-800 dark:bg-blue-800 dark:text-blue-200"
                            : "bg-neutral-100 text-neutral-700 dark:bg-neutral-800 dark:text-neutral-300"
                        }`}
                      >
                        /{suggestion.spec.name}
                      </span>
                      <span className="truncate text-xs text-neutral-500 dark:text-neutral-400">
                        {suggestion.spec.description}
                      </span>
                    </button>
                  ))}
                </div>
                <div className="border-t border-neutral-100 px-4 py-1.5 text-[10px] text-neutral-400 dark:border-neutral-800 dark:text-neutral-500">
                  <kbd className="rounded bg-neutral-100 px-1 py-0.5 font-mono dark:bg-neutral-800">
                    ↵
                  </kbd>{" "}
                  Execute ·{" "}
                  <kbd className="rounded bg-neutral-100 px-1 py-0.5 font-mono dark:bg-neutral-800">
                    Tab
                  </kbd>{" "}
                  Execute ·{" "}
                  <kbd className="rounded bg-neutral-100 px-1 py-0.5 font-mono dark:bg-neutral-800">
                    ↑↓
                  </kbd>{" "}
                  Navigate ·{" "}
                  <kbd className="rounded bg-neutral-100 px-1 py-0.5 font-mono dark:bg-neutral-800">
                    Esc
                  </kbd>{" "}
                  Close
                </div>
              </div>
            </div>
          )}

          {/* File mention popover */}
          {fileMention?.visible && (
            <FileMentionPopover
              query={fileMention.query}
              position={fileMention.cursorPosition}
              onSelect={handleFileSelect}
              onClose={() => setFileMention(null)}
            />
          )}

          <textarea
            ref={textareaRef}
            value={inputValue}
            onChange={handleInputChange}
            onKeyDown={handleKeydown}
            onCompositionStart={handleCompositionStart}
            onCompositionEnd={handleCompositionEnd}
            placeholder={
              isDraftSession
                ? "Type your first message to create the session..."
                : currentSessionId
                  ? "Type a message..."
                  : "Select or create a session to start"
            }
            rows={1}
            disabled={!isInputEnabled}
            className="min-h-[44px] max-h-[200px] w-full resize-none rounded-xl border border-neutral-300 bg-white px-3 py-2.5 text-sm text-neutral-900 placeholder-neutral-400 outline-none transition-colors focus:border-neutral-500 focus:ring-1 focus:ring-neutral-500 disabled:opacity-50 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:placeholder-neutral-500 dark:focus:border-neutral-400"
          />

          {/* Send/Stop button */}
          <div className="absolute bottom-2 right-2 flex items-end">
            {isStreaming ? (
              <button
                onClick={handleStop}
                className="mb-1 flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-full bg-red-100 text-red-600 transition-colors hover:bg-red-200 dark:bg-red-900/30 dark:text-red-400 dark:hover:bg-red-900/50"
                aria-label="Stop streaming"
              >
                <Square className="h-4 w-4" fill="currentColor" />
              </button>
            ) : (
              <button
                onClick={handleSubmit}
                disabled={!inputValue.trim() || !isInputEnabled || isSubmitting}
                className="mb-1 flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-full bg-neutral-900 text-white transition-colors hover:bg-neutral-800 disabled:opacity-50 disabled:hover:bg-neutral-900 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
                aria-label="Send message"
              >
                <ArrowUp className="h-4 w-4" />
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
