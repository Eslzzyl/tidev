import { useEffect, useMemo, useRef, useState } from "react";
import { Check, ChevronDown, CircleStop, ListTodo, LoaderCircle, Send } from "lucide-react";
import { useTranslation } from "react-i18next";

import { commandFragment, getSuggestions } from "../../commands";
import type { Model, TodoItem } from "../../types/api";
import { CommandPopover } from "../CommandPopover";
import { FileMentionPopover } from "../FileMentionPopover";
import { ModelPicker } from "../ModelPicker";

export interface ChatComposerProps {
  draft: string;
  mode: "build" | "plan";
  models: Model[];
  activeModel: Model | undefined;
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
}

export function ChatComposer({
  draft,
  mode,
  models,
  activeModel,
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
}: ChatComposerProps) {
  const { t } = useTranslation();
  const [todoPickerOpen, setTodoPickerOpen] = useState(false);
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(true);
  const [commandIndex, setCommandIndex] = useState(0);
  const [cursorPosition, setCursorPosition] = useState(() => draft.length);
  const composerWrapRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const composingRef = useRef(false);
  const compositionJustCommittedRef = useRef(false);
  const compositionEndTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

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
        setTodoPickerOpen(false);
        setCommandPaletteOpen(false);
        onFileMentionClose();
        return;
      }

      if (!(target instanceof Element) || !target.closest(".composer-menu")) {
        setTodoPickerOpen(false);
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

  const closeOtherMenus = () => {
    setTodoPickerOpen((current) => !current);
    setCommandPaletteOpen(false);
    onFileMentionClose();
  };

  const selectCommand = (suggestion: (typeof commandSuggestions)[number]) => {
    if (commandQuery === null) return;

    const prefix = draft.slice(0, cursorPosition);
    const commandStart = prefix.length - prefix.trimStart().length;
    const commandEnd = commandStart + commandQuery.length + 1;
    const replacement = `/${suggestion.spec.name} `;
    const nextDraft = `${draft.slice(0, commandStart)}${replacement}${draft.slice(commandEnd)}`;
    const nextCursor = commandStart + replacement.length;

    setCursorPosition(nextCursor);
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

  return (
    <div ref={composerWrapRef} className="composer-wrap">
      <div className="composer-toolbar">
        <button
          className={mode === "plan" ? "composer-control plan" : "composer-control build"}
          onClick={() => onModeChange(mode === "plan" ? "build" : "plan")}
        >
          {mode === "plan" ? t("Plan") : t("Build")}
        </button>
        <ModelPicker
          models={models}
          activeModel={activeModel}
          thinkingLevel={thinkingLevel}
          onSelectModel={onSelectModel}
          onSelectThinkingLevel={onSelectThinkingLevel}
          onOpen={() => {
            setTodoPickerOpen(false);
            setCommandPaletteOpen(false);
            onFileMentionClose();
          }}
        />
        <div className="composer-menu">
          <button className="composer-control neutral" onClick={closeOtherMenus}>
            <ListTodo size={13} />
            <span>
              {t("To-Do")}
              {todos.length ? ` (${todos.length})` : ""}
            </span>
            <ChevronDown size={13} />
          </button>
          {todoPickerOpen ? (
            <div className="composer-popover todo-popover">
              {todos.length ? (
                todos.map((todo, index) => (
                  <div className="todo-item" key={`${todo.content}:${index}`}>
                    <span
                      className={todo.status === "completed" ? "todo-check done" : "todo-check"}
                    >
                      {todo.status === "completed" ? <Check size={11} /> : null}
                    </span>
                    <span>{todo.content}</span>
                  </div>
                ))
              ) : (
                <div className="todo-empty">{t("No to-do items in this conversation.")}</div>
              )}
            </div>
          ) : null}
        </div>
        <div className="composer-spacer" />
        <span className="composer-hint">
          {enterToSend ? t("Enter to send · Shift+Enter for newline") : t("Ctrl+Enter to send")}
        </span>
      </div>
      <div className="composer" style={{ position: "relative" }}>
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
                  setCursorPosition(cursor);
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
        <textarea
          ref={textareaRef}
          value={draft}
          onChange={(event) => {
            const value = event.target.value;
            const cursor = event.target.selectionStart ?? value.length;
            setCursorPosition(cursor);
            setCommandPaletteOpen(true);
            onDraftChange(value);
            onFileMentionChange(value, cursor);
          }}
          onSelect={(event) => {
            const textarea = event.target as HTMLTextAreaElement;
            const cursor = textarea.selectionStart ?? textarea.value.length;
            setCursorPosition(cursor);
            setCommandPaletteOpen(true);
            onFileMentionChange(textarea.value, cursor);
          }}
          onClick={(event) => {
            const textarea = event.target as HTMLTextAreaElement;
            const cursor = textarea.selectionStart ?? textarea.value.length;
            setCursorPosition(cursor);
            setCommandPaletteOpen(true);
            onFileMentionChange(textarea.value, cursor);
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
                  return (current + delta + commandSuggestions.length) % commandSuggestions.length;
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
        <button
          className={isBusy ? "send-button stop" : "send-button"}
          disabled={sending || canceling || !selectedSessionId || (!isBusy && !draft.trim())}
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
        </button>
      </div>
    </div>
  );
}
