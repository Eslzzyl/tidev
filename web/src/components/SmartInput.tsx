import { useEffect, useCallback, useState, useRef } from "react";
import { ChevronDown, ArrowUp, Square, Loader2 } from "lucide-react";
import { useSmartInput } from "../hooks/useSmartInput";
import { useAutoResizeTextarea } from "../hooks/useAutoResizeTextarea";
import { FileMentionPopover } from "./chat/FileMentionPopover";
import { commandFragment, getSuggestions } from "../commands";
import { ModelPanel } from "./chat/ModelPanel";
import { api } from "../api/client";
import { formatWorkspace } from "../utils/format";

export interface SmartInputProps {
  /** Called when user submits the input */
  onSubmit: (payload: {
    inputValue: string;
    modelId: string | null;
    providerId: string | null;
    mode: "plan" | "build";
    thinkingLevel: string | null;
  }) => void;

  /** Called when a slash command is executed */
  onSlashCommand?: (command: string) => void;

  /** Placeholder text for the input */
  placeholder?: string;

  /** Whether to use a textarea (multi-line) or input (single-line) */
  multiline?: boolean;

  /** Whether the input is disabled */
  disabled?: boolean;

  /** Whether to show the submit button */
  showSubmitButton?: boolean;

  /** Custom class for the container */
  className?: string;

  /** Custom class for the input element */
  inputClassName?: string;

  /** Initial model ID */
  initialModelId?: string | null;

  /** Initial mode */
  initialMode?: "plan" | "build";

  /** Whether to auto-focus the input on mount */
  autoFocus?: boolean;

  /** Whether this is for a draft session (shows special placeholder) */
  isDraftSession?: boolean;

  /** Current session ID (for context-aware commands) */
  currentSessionId?: string | null;

  /** Whether streaming is in progress (for stop button) */
  isStreaming?: boolean;

  /** Called when stop button is clicked */
  onStop?: () => void;

  /** Workspace root path to display (will show relative to ~ if possible) */
  workspacePath?: string;
}

export function SmartInput({
  onSubmit,
  onSlashCommand,
  placeholder = "Type a message...",
  multiline = false,
  disabled = false,
  showSubmitButton = true,
  className = "",
  inputClassName = "",
  initialModelId = null,
  initialMode = "build",
  autoFocus = false,
  isDraftSession = false,
  isStreaming = false,
  onStop,
  workspacePath,
}: SmartInputProps) {
  const smartInput = useSmartInput({
    initialModelId,
    initialMode,
  });

  const [modelPanelOpen, setModelPanelOpen] = useState(false);

  const {
    inputValue,
    setInputValue,
    isSubmitting,
    setIsSubmitting,
    mode,
    toggleMode,
    selectedModelDisplay,
    selectedProviderId,
    selectedModelId,
    handleModelSelect,
    thinkingOptions,
    selectedThinking,
    setSelectedThinking,
    thinkingDropdownOpen,
    setThinkingDropdownOpen,
    fileMention,
    setFileMention,
    handleFileSelect,
    commandPalette,
    setCommandPalette,
    closeCommandPalette,
    inputRef,
    thinkingDropdownRef,
    getSubmitPayload,
  } = smartInput;

  // IME composition guards.
  //
  // Cross-browser fix for IME composition + Enter-to-submit.
  // See MessageInput.tsx for the detailed explanation of the Safari
  // compositionend-before-keydown ordering issue (WebKit bug 165004).
  //
  // Pattern: maintain a custom ref set on compositionend, cleared via
  // setTimeout (macrotask). Also check e.isComposing for Chrome/Firefox.
  const composingRef = useRef(false);
  const compositionJustCommittedRef = useRef(false);
  const compositionEndTimerRef = useRef<
    ReturnType<typeof setTimeout> | undefined
  >(undefined);

  // Clean up the timer on unmount.
  useEffect(() => {
    return () => clearTimeout(compositionEndTimerRef.current);
  }, []);

  function handleCompositionStart() {
    composingRef.current = true;
    compositionJustCommittedRef.current = false;
    clearTimeout(compositionEndTimerRef.current);
  }

  function handleCompositionEnd() {
    composingRef.current = false;
    compositionJustCommittedRef.current = true;
    clearTimeout(compositionEndTimerRef.current);
    compositionEndTimerRef.current = setTimeout(() => {
      compositionJustCommittedRef.current = false;
    }, 0);
  }

  // Auto-focus on mount
  useEffect(() => {
    if (autoFocus && inputRef.current) {
      inputRef.current.focus();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoFocus]);

  // Handle submit
  const handleSubmit = useCallback(async () => {
    if (!inputValue.trim() || isSubmitting || disabled) return;

    setIsSubmitting(true);
    try {
      const payload = getSubmitPayload();
      await onSubmit(payload);
      setInputValue("");
    } finally {
      setIsSubmitting(false);
    }
  }, [
    inputValue,
    isSubmitting,
    disabled,
    getSubmitPayload,
    onSubmit,
    setInputValue,
    setIsSubmitting,
  ]);

  // Handle command execution with callback
  const executeCommand = useCallback(
    (name: string) => {
      setInputValue("");
      closeCommandPalette();

      // Handle commands that don't need session
      if (name === "new" || name === "clear") {
        // Navigate to welcome - handled by consumer
        onSlashCommand?.("new");
        return;
      }

      // For other commands, delegate to parent
      onSlashCommand?.(name);
    },
    [setInputValue, closeCommandPalette, onSlashCommand],
  );

  // Enhanced keydown handler with command execution
  const enhancedHandleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement | HTMLInputElement>) => {
      // Handle command palette navigation
      if (commandPalette.visible) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          const newIndex =
            (commandPalette.selectedIndex + 1) %
            commandPalette.suggestions.length;
          setCommandPalette({
            ...commandPalette,
            selectedIndex: newIndex,
          });
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          const newIndex =
            commandPalette.selectedIndex === 0
              ? commandPalette.suggestions.length - 1
              : commandPalette.selectedIndex - 1;
          setCommandPalette({
            ...commandPalette,
            selectedIndex: newIndex,
          });
          return;
        }
        if (e.key === "Enter" || e.key === "Tab") {
          e.preventDefault();
          const suggestion =
            commandPalette.suggestions[commandPalette.selectedIndex];
          if (suggestion) {
            executeCommand(suggestion.spec.name);
          }
          return;
        }
        if (e.key === "Escape") {
          e.preventDefault();
          closeCommandPalette();
          return;
        }
        return;
      }

      // Handle file mention escape
      if (fileMention?.visible && e.key === "Escape") {
        e.preventDefault();
        setFileMention(null);
        return;
      }

      // Handle Tab to toggle mode
      if (e.key === "Tab") {
        e.preventDefault();
        toggleMode();
        return;
      }

      // Handle submit on Enter (skip during IME composition)
      if (
        e.key === "Enter" &&
        !e.shiftKey &&
        !e.nativeEvent.isComposing &&
        !composingRef.current &&
        !compositionJustCommittedRef.current
      ) {
        e.preventDefault();
        if (!isSubmitting && !disabled) {
          handleSubmit();
        }
        return;
      }
    },
    [
      commandPalette,
      fileMention,
      isSubmitting,
      disabled,
      handleSubmit,
      executeCommand,
      closeCommandPalette,
      setCommandPalette,
      setFileMention,
      toggleMode,
    ],
  );

  // Handle input change with command detection
  const enhancedHandleInputChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement | HTMLInputElement>) => {
      const newValue = e.target.value;
      const input = e.target;
      setInputValue(newValue);

      // Get cursor position
      const cursorPosition =
        "selectionStart" in input ? (input.selectionStart ?? 0) : 0;

      // Check for @ mention
      const textBeforeCursor = newValue.slice(0, cursorPosition);
      const atMatch = textBeforeCursor.match(/@([^\s]*)$/);

      if (atMatch) {
        const query = atMatch[1] || "";
        const atPosition = textBeforeCursor.lastIndexOf("@");

        // Get cursor coordinates
        if ("getBoundingClientRect" in input) {
          const rect = input.getBoundingClientRect();
          setFileMention({
            visible: true,
            query,
            atPosition,
            cursorPosition: { x: rect.left + 20, y: rect.top },
          });
        }
      } else {
        setFileMention(null);
      }

      // Check for / command
      const fragment = commandFragment(newValue);
      if (fragment !== null) {
        const suggestions = getSuggestions(fragment);
        if (suggestions.length > 0) {
          if ("getBoundingClientRect" in input) {
            const rect = input.getBoundingClientRect();
            setCommandPalette({
              visible: true,
              selectedIndex: 0,
              suggestions,
              position: { x: rect.left, y: rect.top },
            });
          }
        } else {
          closeCommandPalette();
        }
      } else {
        closeCommandPalette();
      }
    },
    [setInputValue, setFileMention, setCommandPalette, closeCommandPalette],
  );

  // Compute final placeholder
  const finalPlaceholder = isDraftSession
    ? "Type your first message to create the session..."
    : placeholder;

  // Determine if input is effectively enabled
  const isInputEnabled = !disabled;

  // Auto-resize textarea when in multiline mode (up to 200px, then scrolls)
  useAutoResizeTextarea(
    inputRef as React.RefObject<HTMLTextAreaElement | null>,
    inputValue,
    200,
  );

  return (
    <div className={`relative ${className}`}>
      {/* Command Palette */}
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
                    inputRef.current?.focus();
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

      {/* File Mention Popover */}
      {fileMention?.visible && (
        <FileMentionPopover
          query={fileMention.query}
          position={fileMention.cursorPosition}
          onSelect={handleFileSelect}
          onClose={() => setFileMention(null)}
        />
      )}

      {/* Toolbar */}
      <div className="mb-2 flex items-center gap-2 px-1">
        {/* Mode Toggle */}
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

        {/* Model Selector - opens ModelPanel */}
        <button
          onClick={() => setModelPanelOpen(true)}
          className="flex items-center gap-1 rounded bg-neutral-100 px-2 py-1 text-xs text-neutral-700 hover:bg-neutral-200 dark:bg-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-700"
        >
          <span className="max-w-[180px] truncate">
            {selectedModelDisplay?.display_name
              ? `${selectedModelDisplay.provider_name}/${selectedModelDisplay.display_name}`
              : "Select model"}
          </span>
          <ChevronDown className="h-3 w-3" />
        </button>

        {/* Model Panel */}
        <ModelPanel
          isOpen={modelPanelOpen}
          onClose={() => setModelPanelOpen(false)}
          currentModelId={selectedModelDisplay?.id || null}
          currentProviderId={selectedModelDisplay?.provider_id || null}
          onModelChange={handleModelSelect}
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
                      // Persist thinking level preference to backend
                      if (selectedProviderId && selectedModelId) {
                        api
                          .setModelThinkingLevel({
                            provider_id: selectedProviderId,
                            model_id: selectedModelId,
                            thinking_level: option.value,
                          })
                          .catch(() => {});
                      }
                    }}
                    className={`flex w-full px-3 py-2 text-left text-xs hover:bg-neutral-100 dark:hover:bg-neutral-800 ${
                      selectedThinking === option.value
                        ? "bg-neutral-100 dark:bg-neutral-800 font-medium"
                        : ""
                    }`}
                  >
                    {option.label}
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
      </div>

      {/* Input Container */}
      <div className="relative rounded-2xl border border-neutral-200 bg-white shadow-lg transition-all duration-200 ease-smooth focus-within:border-neutral-400 focus-within:shadow-[0_0_0_3px_rgba(0,0,0,0.04)] dark:border-neutral-800 dark:bg-neutral-900 dark:focus-within:border-neutral-600 dark:focus-within:shadow-[0_0_0_3px_rgba(255,255,255,0.04)]">
        {/* Input Element */}
        {multiline ? (
          <textarea
            ref={inputRef as React.RefObject<HTMLTextAreaElement>}
            value={inputValue}
            onChange={enhancedHandleInputChange}
            onKeyDown={enhancedHandleKeyDown}
            onCompositionStart={handleCompositionStart}
            onCompositionEnd={handleCompositionEnd}
            placeholder={finalPlaceholder}
            rows={1}
            disabled={!isInputEnabled}
            className={`min-h-[44px] w-full resize-none rounded-2xl bg-transparent px-4 py-3 pr-12 text-base text-neutral-900 placeholder-neutral-400 outline-none disabled:opacity-50 dark:text-neutral-100 dark:placeholder-neutral-500 ${inputClassName}`}
          />
        ) : (
          <input
            ref={inputRef as React.RefObject<HTMLInputElement>}
            type="text"
            value={inputValue}
            onChange={enhancedHandleInputChange}
            onKeyDown={enhancedHandleKeyDown}
            onCompositionStart={handleCompositionStart}
            onCompositionEnd={handleCompositionEnd}
            placeholder={finalPlaceholder}
            disabled={!isInputEnabled}
            className={`w-full rounded-2xl bg-transparent px-4 py-3 pr-12 text-base text-neutral-900 placeholder-neutral-400 outline-none disabled:opacity-50 dark:text-neutral-100 dark:placeholder-neutral-500 ${inputClassName}`}
          />
        )}

        {/* Submit/Stop Button */}
        {showSubmitButton && (
          <div className="absolute right-2 top-1/2 -translate-y-1/2">
            {isStreaming ? (
              <button
                onClick={onStop}
                className="flex h-8 w-8 items-center justify-center rounded-full bg-red-100 text-red-600 transition-colors hover:bg-red-200 dark:bg-red-900/30 dark:text-red-400 dark:hover:bg-red-900/50"
                aria-label="Stop streaming"
              >
                <Square className="h-4 w-4" fill="currentColor" />
              </button>
            ) : isSubmitting ? (
              <div className="flex h-8 w-8 items-center justify-center">
                <Loader2 className="h-5 w-5 animate-spin text-neutral-400" />
              </div>
            ) : (
              <button
                onClick={handleSubmit}
                disabled={!inputValue.trim() || !isInputEnabled}
                className="flex h-8 w-8 items-center justify-center rounded-full bg-neutral-900 text-white transition-colors hover:bg-neutral-800 disabled:opacity-50 disabled:hover:bg-neutral-900 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
                aria-label="Send message"
              >
                <ArrowUp className="h-4 w-4" />
              </button>
            )}
          </div>
        )}
      </div>

      {/* Workspace path display */}
      {workspacePath && (
        <span className="text-xs text-neutral-400 dark:text-neutral-500 truncate max-w-[200px]">
          {formatWorkspace(workspacePath)}
        </span>
      )}
    </div>
  );
}
