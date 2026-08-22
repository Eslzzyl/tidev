import { useEffect, useMemo, useRef, useState } from "react";
import { Clock3, LoaderCircle, Send } from "lucide-react";
import { useTranslation } from "react-i18next";

import { commandFragment, getSuggestions } from "../../commands";
import type { Model, Session } from "../../types/api";
import { CommandPopover } from "../CommandPopover";
import { FileMentionPopover } from "../FileMentionPopover";
import { ModelPicker } from "../ModelPicker";
import { formatDate } from "../../utils/chat";

export interface WelcomePageProps {
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
  fileMention: { query: string; atPos: number } | null;
  fileMentionIndex: number;
  onChangeDraft: (value: string) => void;
  onModeChange: (mode: "build" | "plan") => void;
  onSelectSession: (sessionId: string) => void;
  onSelectModel: (model: Model) => void;
  onSelectThinkingLevel: (level: string) => void;
  onSubmit: () => void;
  onFileMentionChange: (text: string, cursor: number) => void;
  onFileMentionIndexChange: (index: number) => void;
  onFileSelect: (path: string) => number | undefined;
  onFileMentionClose: () => void;
}

export function WelcomePage({
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
  fileMention,
  fileMentionIndex,
  onChangeDraft,
  onModeChange,
  onSelectSession,
  onSelectModel,
  onSelectThinkingLevel,
  onSubmit,
  onFileMentionChange,
  onFileMentionIndexChange,
  onFileSelect,
  onFileMentionClose,
}: WelcomePageProps) {
  const { t } = useTranslation();
  const compositionRef = useRef(false);
  const composerRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(true);
  const [commandIndex, setCommandIndex] = useState(0);
  const [cursorPosition, setCursorPosition] = useState(() => draft.length);

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
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.style.height = "auto";
    const height = Math.min(textarea.scrollHeight, 200);
    textarea.style.height = `${height}px`;
    textarea.style.overflowY = textarea.scrollHeight > 200 ? "auto" : "hidden";
  }, [draft]);

  useEffect(() => {
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node) || !composerRef.current?.contains(target)) {
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
    onChangeDraft(nextDraft);
    onFileMentionChange(nextDraft, nextCursor);
    requestAnimationFrame(() => {
      const textarea = textareaRef.current;
      if (!textarea) return;
      textarea.focus();
      textarea.setSelectionRange(nextCursor, nextCursor);
    });
  };

  return (
    <section className="welcome-page">
      <div className="welcome-heading">
        <div className="welcome-logo">t</div>
        <h1>tidev</h1>
        <p>{t("Your intelligent coding assistant")}</p>
      </div>
      <div ref={composerRef} className="welcome-composer">
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
              onChangeDraft(value);
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
              compositionRef.current = true;
            }}
            onCompositionEnd={() => {
              compositionRef.current = false;
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
            placeholder={t("What would you like to work on?")}
            rows={2}
          />
        </div>
        <div className="welcome-composer-footer">
          <div className="welcome-controls">
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
                setCommandPaletteOpen(false);
                onFileMentionClose();
              }}
            />
          </div>
          <button
            className="send-button"
            disabled={!draft.trim() || loading || sending}
            onClick={onSubmit}
            title={t("Start conversation")}
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
            <span>{t("Recent Sessions")}</span>
          </div>
          <div className="recent-session-grid">
            {sessions.slice(0, 5).map((session) => (
              <button
                className="recent-session"
                key={session.session_id}
                onClick={() => onSelectSession(session.session_id)}
              >
                <span>{session.title || t("Untitled conversation")}</span>
                <time>{formatDate(session.updated_at)}</time>
              </button>
            ))}
          </div>
        </div>
      ) : null}
    </section>
  );
}
