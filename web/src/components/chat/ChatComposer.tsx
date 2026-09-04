import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ClipboardEvent } from "react";
import { Check, CircleStop, LoaderCircle, Send } from "lucide-react";
import { useTranslation } from "react-i18next";

import { commandFragment, getSuggestions } from "../../commands";
import type { MessageRecord, Model, TodoItem } from "../../types/api";
import { CommandPopover } from "../CommandPopover";
import { FileMentionPopover } from "../FileMentionPopover";
import { ModelPicker } from "../ModelPicker";
import { ImageAttachmentStrip } from "./ImageAttachments";
import { SubagentStatusIndicator } from "./SubagentStatusIndicator";
import { TokenUsageIndicator } from "./TokenUsageIndicator";
import { pastedImageFiles, type PendingImage } from "../../utils/imageAttachments";
import { Button, IconButton, Textarea } from "../ui";

function isTodoCompleted(todo: TodoItem) {
  return todo.status === "completed";
}

function TodoProgressCard({ todos }: { todos: TodoItem[] }) {
  const { t } = useTranslation();
  if (todos.length === 0) return null;

  const activeIndex = todos.findIndex((todo) => todo.status === "in_progress");
  const currentIndex =
    activeIndex >= 0 ? activeIndex : todos.findIndex((todo) => !isTodoCompleted(todo));
  const displayedIndex = currentIndex >= 0 ? currentIndex : todos.length - 1;

  return (
    <div
      className="composer-todo-progress"
      tabIndex={0}
      role="group"
      aria-label={t("Step {{current}} / {{total}}", {
        current: displayedIndex + 1,
        total: todos.length,
      })}
    >
      <div className="composer-todo-card">
        <div className="composer-todo-list">
          {todos.map((todo, index) => {
            const completed = isTodoCompleted(todo);
            const active = index === displayedIndex && !completed;
            const className = [
              "composer-todo-entry",
              completed ? "is-completed" : "",
              active ? "is-active" : "",
            ]
              .filter(Boolean)
              .join(" ");

            return (
              <div className={className} key={`${todo.content}:${index}`}>
                <span className="composer-todo-entry-check" aria-hidden="true">
                  {completed ? <Check size={12} /> : null}
                </span>
                <span>{todo.content}</span>
              </div>
            );
          })}
        </div>
      </div>
      <div className="composer-todo-trigger">
        <span className="composer-todo-trigger-dot" aria-hidden="true" />
        <span>
          {t("Step {{current}} / {{total}}", {
            current: displayedIndex + 1,
            total: todos.length,
          })}
        </span>
      </div>
    </div>
  );
}

export interface ChatComposerProps {
  draft: string;
  mode: "build" | "plan";
  messages: MessageRecord[];
  models: Model[];
  activeModel: Model | undefined;
  contextWindow?: number;
  thinkingLevel: string | undefined;
  todos: TodoItem[];
  enterToSend: boolean;
  isBusy: boolean;
  sending: boolean;
  canceling: boolean;
  selectedSessionId: string | null;
  fileMention: { query: string; atPos: number } | null;
  fileMentionIndex: number;
  onDraftChange: (value: string) => void;
  onModeChange: (mode: "build" | "plan") => void;
  onSelectModel: (model: Model) => void;
  onSelectThinkingLevel: (level: string) => void;
  onSubmit: () => void;
  onCancel: () => void;
  onFileMentionChange: (text: string, cursor: number) => void;
  onFileMentionIndexChange: (index: number) => void;
  onFileSelect: (path: string) => number | undefined;
  onFileMentionClose: () => void;
  pendingImages: PendingImage[];
  onImagesPasted: (files: File[]) => void;
  onRemoveImage: (id: string) => void;
  initialSelection?: { start: number; end: number; direction: "forward" | "backward" | "none" };
  onSelectionChange?: (selection: {
    start: number;
    end: number;
    direction: "forward" | "backward" | "none";
  }) => void;
  autoFocus?: boolean;
  onAutoFocus?: () => void;
}

export function ChatComposer({
  draft,
  mode,
  messages,
  models,
  activeModel,
  contextWindow,
  thinkingLevel,
  todos,
  enterToSend,
  isBusy,
  sending,
  canceling,
  selectedSessionId,
  fileMention,
  fileMentionIndex,
  onDraftChange,
  onModeChange,
  onSelectModel,
  onSelectThinkingLevel,
  onSubmit,
  onCancel,
  onFileMentionChange,
  onFileMentionIndexChange,
  onFileSelect,
  onFileMentionClose,
  pendingImages,
  onImagesPasted,
  onRemoveImage,
  initialSelection: initialSelectionProp,
  onSelectionChange,
  autoFocus = false,
  onAutoFocus,
}: ChatComposerProps) {
  const { t } = useTranslation();
  const [initialSelection] = useState(() => initialSelectionProp);
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(true);
  const [commandIndex, setCommandIndex] = useState(0);
  const [cursorPosition, setCursorPosition] = useState(() => initialSelection?.end ?? draft.length);
  const composerWrapRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const composingRef = useRef(false);
  const compositionJustCommittedRef = useRef(false);
  const compositionEndTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useLayoutEffect(() => {
    if (!autoFocus) return;
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.focus();
    onAutoFocus?.();
  }, [autoFocus, onAutoFocus]);

  useLayoutEffect(() => {
    if (!initialSelection) return;
    const textarea = textareaRef.current;
    if (!textarea) return;
    const start = Math.min(initialSelection.start, textarea.value.length);
    const end = Math.min(Math.max(initialSelection.end, start), textarea.value.length);
    textarea.setSelectionRange(start, end, initialSelection.direction);
    setCursorPosition(end);
  }, [initialSelection]);

  useEffect(() => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.style.height = "auto";
    const height = Math.min(textarea.scrollHeight, 200);
    textarea.style.height = `${height}px`;
    textarea.style.overflowY = textarea.scrollHeight > 200 ? "auto" : "hidden";
  }, [draft]);

  useEffect(
    () => () => {
      if (compositionEndTimerRef.current) clearTimeout(compositionEndTimerRef.current);
    },
    [],
  );

  const commandQuery = commandFragment(draft, cursorPosition);
  const commandSuggestions = useMemo(
    () => (commandQuery === null ? [] : getSuggestions(commandQuery)),
    [commandQuery],
  );
  const commandPaletteVisible = commandPaletteOpen && commandSuggestions.length > 0 && !fileMention;

  useEffect(() => {
    setCommandIndex(0);
  }, [commandQuery]);

  useEffect(() => {
    if (cursorPosition <= draft.length) return;
    setCursorPosition(draft.length);
  }, [cursorPosition, draft.length]);

  useEffect(() => {
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;

      const composerWrap = composerWrapRef.current;
      if (!composerWrap?.contains(target)) {
        setCommandPaletteOpen(false);
        onFileMentionClose();
        return;
      }

      if (
        !(target instanceof Element) ||
        (!target.closest(".command-popover") && !target.closest("textarea"))
      ) {
        setCommandPaletteOpen(false);
      }
    };

    document.addEventListener("pointerdown", handlePointerDown);
    return () => document.removeEventListener("pointerdown", handlePointerDown);
  }, [onFileMentionClose]);

  const updateSelection = (
    start: number,
    end = start,
    direction: "forward" | "backward" | "none" = "none",
  ) => {
    setCursorPosition(end);
    onSelectionChange?.({ start, end, direction });
  };

  const selectCommand = (suggestion: (typeof commandSuggestions)[number]) => {
    if (commandQuery === null) return;

    const prefix = draft.slice(0, cursorPosition);
    const commandStart = prefix.length - prefix.trimStart().length;
    const commandEnd = commandStart + commandQuery.length + 1;
    const replacement = `/${suggestion.spec.name} `;
    const nextDraft = `${draft.slice(0, commandStart)}${replacement}${draft.slice(commandEnd)}`;
    const nextCursor = commandStart + replacement.length;

    updateSelection(nextCursor);
    setCommandPaletteOpen(false);
    onDraftChange(nextDraft);
    onFileMentionChange(nextDraft, nextCursor);
    requestAnimationFrame(() => {
      const textarea = textareaRef.current;
      if (!textarea) return;
      textarea.focus();
      textarea.setSelectionRange(nextCursor, nextCursor);
    });
  };

  const handlePaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = pastedImageFiles(event.clipboardData);
    if (files.length === 0) return;

    event.preventDefault();
    const pastedText = event.clipboardData.getData("text/plain");
    if (pastedText) {
      const textarea = event.currentTarget;
      const start = textarea.selectionStart ?? draft.length;
      const end = textarea.selectionEnd ?? start;
      const nextDraft = `${draft.slice(0, start)}${pastedText}${draft.slice(end)}`;
      const nextCursor = start + pastedText.length;
      updateSelection(nextCursor);
      setCommandPaletteOpen(true);
      onDraftChange(nextDraft);
      onFileMentionChange(nextDraft, nextCursor);
      requestAnimationFrame(() => {
        textarea.focus();
        textarea.setSelectionRange(nextCursor, nextCursor);
      });
    }
    onImagesPasted(files);
  };

  return (
    <div ref={composerWrapRef} className="composer-wrap">
      <TodoProgressCard todos={todos} />
      <div className="welcome-composer">
        <div className="welcome-input">
          {fileMention ? (
            <FileMentionPopover
              query={fileMention.query}
              selectedIndex={fileMentionIndex}
              onSelectedIndexChange={onFileMentionIndexChange}
              onSelect={(path) => {
                const cursor = onFileSelect(path);
                requestAnimationFrame(() => {
                  const textarea = textareaRef.current;
                  if (!textarea) return;
                  textarea.focus();
                  if (cursor !== undefined) {
                    updateSelection(cursor);
                    textarea.setSelectionRange(cursor, cursor);
                  }
                });
              }}
              onClose={onFileMentionClose}
            />
          ) : commandPaletteVisible ? (
            <CommandPopover
              suggestions={commandSuggestions}
              selectedIndex={commandIndex}
              onSelectedIndexChange={setCommandIndex}
              onSelect={selectCommand}
            />
          ) : null}
          <ImageAttachmentStrip
            images={pendingImages}
            onRemove={onRemoveImage}
            disabled={sending || canceling}
          />
          <Textarea
            ref={textareaRef}
            value={draft}
            onPaste={handlePaste}
            onChange={(event) => {
              const value = event.target.value;
              const start = event.target.selectionStart ?? value.length;
              const end = event.target.selectionEnd ?? start;
              updateSelection(start, end, event.target.selectionDirection);
              setCommandPaletteOpen(true);
              onDraftChange(value);
              onFileMentionChange(value, end);
            }}
            onSelect={(event) => {
              const textarea = event.target as HTMLTextAreaElement;
              const start = textarea.selectionStart ?? textarea.value.length;
              const end = textarea.selectionEnd ?? start;
              updateSelection(start, end, textarea.selectionDirection);
              setCommandPaletteOpen(true);
              onFileMentionChange(textarea.value, end);
            }}
            onClick={(event) => {
              const textarea = event.target as HTMLTextAreaElement;
              const start = textarea.selectionStart ?? textarea.value.length;
              const end = textarea.selectionEnd ?? start;
              updateSelection(start, end, textarea.selectionDirection);
              setCommandPaletteOpen(true);
              onFileMentionChange(textarea.value, end);
            }}
            onCompositionStart={() => {
              composingRef.current = true;
              compositionJustCommittedRef.current = false;
              if (compositionEndTimerRef.current) clearTimeout(compositionEndTimerRef.current);
            }}
            onCompositionEnd={() => {
              composingRef.current = false;
              compositionJustCommittedRef.current = true;
              if (compositionEndTimerRef.current) clearTimeout(compositionEndTimerRef.current);
              compositionEndTimerRef.current = setTimeout(() => {
                compositionJustCommittedRef.current = false;
              }, 0);
            }}
            onKeyDown={(event) => {
              if (commandPaletteVisible) {
                if (event.key === "Escape") {
                  event.preventDefault();
                  setCommandPaletteOpen(false);
                  return;
                }
                if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                  event.preventDefault();
                  setCommandIndex((current) => {
                    const delta = event.key === "ArrowDown" ? 1 : -1;
                    return (
                      (current + delta + commandSuggestions.length) % commandSuggestions.length
                    );
                  });
                  return;
                }
                if (event.key === "Enter" || event.key === "Tab") {
                  event.preventDefault();
                  const selected = commandSuggestions[commandIndex];
                  if (selected) selectCommand(selected);
                  return;
                }
              }
              if (fileMention) {
                if (event.key === "Escape") {
                  event.preventDefault();
                  onFileMentionClose();
                  return;
                }
                if (event.key === "Enter" || event.key === "Tab") {
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
                onModeChange(mode === "plan" ? "build" : "plan");
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
                onSubmit();
              }
            }}
            placeholder={t("Ask tidev to inspect, plan, or change your workspace…")}
            rows={2}
          />
        </div>
        <div className="welcome-composer-footer">
          <div className="welcome-controls">
            <Button
              className={`composer-control ${mode === "plan" ? "plan" : "build"}`}
              onClick={() => onModeChange(mode === "plan" ? "build" : "plan")}
              variant="ghost"
              size="sm"
            >
              {mode === "plan" ? t("Plan") : t("Build")}
            </Button>
            <ModelPicker
              models={models}
              activeModel={activeModel}
              thinkingLevel={thinkingLevel}
              onSelectModel={onSelectModel}
              onSelectThinkingLevel={onSelectThinkingLevel}
              onOpen={() => {
                setCommandPaletteOpen(false);
                onFileMentionClose();
              }}
            />
            <SubagentStatusIndicator />
            <TokenUsageIndicator messages={messages} contextWindow={contextWindow} />
          </div>
          <IconButton
            label={isBusy ? t("Stop current turn") : t("Send prompt")}
            size="md"
            variant={isBusy ? "danger" : "primary"}
            className={isBusy ? "send-button stop" : "send-button"}
            disabled={
              sending ||
              canceling ||
              !selectedSessionId ||
              (!isBusy && !draft.trim() && pendingImages.length === 0)
            }
            onClick={() => (isBusy ? onCancel() : onSubmit())}
            title={isBusy ? t("Stop current turn") : t("Send prompt")}
          >
            {sending || canceling ? (
              <LoaderCircle className="spin" size={17} />
            ) : isBusy ? (
              <CircleStop size={17} />
            ) : (
              <Send size={17} />
            )}
          </IconButton>
        </div>
      </div>
    </div>
  );
}
