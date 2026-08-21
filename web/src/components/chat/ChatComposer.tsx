import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown, CircleStop, ListTodo, LoaderCircle, Send } from "lucide-react";

import type { Model, TodoItem } from "../../types/api";
import { FileMentionPopover } from "../FileMentionPopover";
import { formatThinkingLevel } from "../../utils/chat";

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
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const [thinkingPickerOpen, setThinkingPickerOpen] = useState(false);
  const [todoPickerOpen, setTodoPickerOpen] = useState(false);
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

  const closeOtherMenus = (menu: "model" | "thinking" | "todo") => {
    setModelPickerOpen(menu === "model" ? (current) => !current : false);
    setThinkingPickerOpen(menu === "thinking" ? (current) => !current : false);
    setTodoPickerOpen(menu === "todo" ? (current) => !current : false);
  };

  return (
    <div className="composer-wrap">
      <div className="composer-toolbar">
        <button
          className={mode === "plan" ? "composer-control plan" : "composer-control build"}
          onClick={() => onModeChange(mode === "plan" ? "build" : "plan")}
        >
          {mode === "plan" ? "Plan" : "Build"}
        </button>
        <div className="composer-menu">
          <button className="composer-control neutral" onClick={() => closeOtherMenus("model")}>
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
                  className={model.active ? "composer-option selected" : "composer-option"}
                  disabled={!model.connected}
                  onClick={() => {
                    onSelectModel(model);
                    setModelPickerOpen(false);
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
              onClick={() => closeOtherMenus("thinking")}
            >
              <span>{formatThinkingLevel(thinkingLevel ?? activeModel.thinking_level)}</span>
              <ChevronDown size={13} />
            </button>
            {thinkingPickerOpen ? (
              <div className="composer-popover thinking-popover">
                {activeModel.thinking_levels.map((level) => (
                  <button
                    key={level}
                    className={
                      thinkingLevel === level ? "composer-option selected" : "composer-option"
                    }
                    onClick={() => {
                      onSelectThinkingLevel(level);
                      setThinkingPickerOpen(false);
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
          <button className="composer-control neutral" onClick={() => closeOtherMenus("todo")}>
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
                      className={todo.status === "completed" ? "todo-check done" : "todo-check"}
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
          {enterToSend ? "Enter to send · Shift+Enter for newline" : "Ctrl+Enter to send"}
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
                if (cursor !== undefined) textarea.setSelectionRange(cursor, cursor);
              });
            }}
            onClose={onFileMentionClose}
          />
        ) : null}
        <textarea
          ref={textareaRef}
          value={draft}
          onChange={(event) => {
            const value = event.target.value;
            const cursor = event.target.selectionStart ?? value.length;
            onDraftChange(value);
            onFileMentionChange(value, cursor);
          }}
          onSelect={(event) => {
            const textarea = event.target as HTMLTextAreaElement;
            onFileMentionChange(textarea.value, textarea.selectionStart ?? textarea.value.length);
          }}
          onClick={(event) => {
            const textarea = event.target as HTMLTextAreaElement;
            onFileMentionChange(textarea.value, textarea.selectionStart ?? textarea.value.length);
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
          placeholder="Ask tidev to inspect, plan, or change your workspace…"
          rows={3}
        />
        <button
          className={isBusy ? "send-button stop" : "send-button"}
          disabled={sending || canceling || !selectedSessionId || (!isBusy && !draft.trim())}
          onClick={() => (isBusy ? onCancel() : onSubmit())}
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
  );
}
