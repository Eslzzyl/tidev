import { useEffect, useMemo, useRef, useState, type ClipboardEvent } from "react";
import { ChevronDown, Folder, GitBranch, LoaderCircle, Send } from "lucide-react";
import { useTranslation } from "react-i18next";

import { api } from "../../api/client";
import { commandFragment, getSuggestions } from "../../commands";
import { useWorkspace } from "../../hooks/workspaceQueries";
import type { Model, WorkspaceContext } from "../../types/api";
import { CommandPopover } from "../CommandPopover";
import { FileMentionPopover } from "../FileMentionPopover";
import { ModelPicker } from "../ModelPicker";
import { ImageAttachmentStrip } from "./ImageAttachments";
import { pastedImageFiles, type PendingImage } from "../../utils/imageAttachments";
import { Button, IconButton, Input, Textarea } from "../ui";

export interface WelcomePageProps {
  draft: string;
  error: string | null;
  loading: boolean;
  mode: "build" | "plan";
  enterToSend: boolean;
  sending: boolean;
  models: Model[];
  activeModel: Model | undefined;
  thinkingLevel: string | undefined;
  fileMention: { query: string; atPos: number } | null;
  fileMentionIndex: number;
  recentWorkspaceRoots: string[];
  onChangeDraft: (value: string) => void;
  onModeChange: (mode: "build" | "plan") => void;
  onSelectModel: (model: Model) => void;
  onSelectThinkingLevel: (level: string) => void;
  onSubmit: (workspaceRoot: string) => void;
  onFileMentionChange: (text: string, cursor: number) => void;
  onFileMentionIndexChange: (index: number) => void;
  onFileSelect: (path: string) => number | undefined;
  onFileMentionClose: () => void;
  pendingImages: PendingImage[];
  onImagesPasted: (files: File[]) => void;
  onRemoveImage: (id: string) => void;
}

export function WelcomePage({
  draft,
  error,
  loading,
  mode,
  enterToSend,
  sending,
  models,
  activeModel,
  thinkingLevel,
  fileMention,
  fileMentionIndex,
  recentWorkspaceRoots,
  onChangeDraft,
  onModeChange,
  onSelectModel,
  onSelectThinkingLevel,
  onSubmit,
  onFileMentionChange,
  onFileMentionIndexChange,
  onFileSelect,
  onFileMentionClose,
  pendingImages,
  onImagesPasted,
  onRemoveImage,
}: WelcomePageProps) {
  const { t } = useTranslation();
  const compositionRef = useRef(false);
  const compositionJustCommittedRef = useRef(false);
  const compositionEndTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const composerRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const workspacePickerRef = useRef<HTMLDivElement>(null);
  const workspacePathInputRef = useRef<HTMLInputElement>(null);
  const { data: defaultWorkspace } = useWorkspace();

  useEffect(
    () => () => {
      if (compositionEndTimerRef.current) clearTimeout(compositionEndTimerRef.current);
    },
    [],
  );
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(true);
  const [commandIndex, setCommandIndex] = useState(0);
  const [cursorPosition, setCursorPosition] = useState(() => draft.length);
  const [selectedWorkspaceRoot, setSelectedWorkspaceRoot] = useState<string | null>(null);
  const [workspaceContext, setWorkspaceContext] = useState<WorkspaceContext | null>(null);
  const [workspacePickerOpen, setWorkspacePickerOpen] = useState(false);
  const [workspacePath, setWorkspacePath] = useState("");
  const [workspaceCompletions, setWorkspaceCompletions] = useState<string[]>([]);
  const [workspaceParent, setWorkspaceParent] = useState<string | null>(null);
  const [workspaceCompletionIndex, setWorkspaceCompletionIndex] = useState(-1);
  const [workspaceLoading, setWorkspaceLoading] = useState(false);
  const [workspacePickerError, setWorkspacePickerError] = useState<string | null>(null);

  const activeWorkspaceRoot = selectedWorkspaceRoot ?? defaultWorkspace?.workspace_root ?? "";
  const workspacePathIsCompletable =
    workspacePath.startsWith("/") || workspacePath === "~" || workspacePath.startsWith("~/");
  const activeWorkspaceContext =
    workspaceContext?.workspace_root === activeWorkspaceRoot ? workspaceContext : null;
  const workspaceLabel =
    activeWorkspaceContext?.workspace_name ??
    activeWorkspaceRoot.split("/").filter(Boolean).pop() ??
    t("Workspace");
  const recentDirectories = useMemo(
    () =>
      [...new Set(recentWorkspaceRoots)]
        .filter((workspaceRoot) => workspaceRoot !== activeWorkspaceRoot)
        .slice(0, 5),
    [activeWorkspaceRoot, recentWorkspaceRoots],
  );

  const commandQuery = commandFragment(draft, cursorPosition);
  const commandSuggestions = useMemo(
    () => (commandQuery === null ? [] : getSuggestions(commandQuery)),
    [commandQuery],
  );
  const commandPaletteVisible = commandPaletteOpen && commandSuggestions.length > 0 && !fileMention;

  useEffect(() => {
    if (!activeWorkspaceRoot || activeWorkspaceContext) return;
    let cancelled = false;

    void api
      .getWorkspaceContext(activeWorkspaceRoot)
      .then((context) => {
        if (!cancelled) setWorkspaceContext(context);
      })
      .catch(() => {
        if (!cancelled) setWorkspaceContext(null);
      });

    return () => {
      cancelled = true;
    };
  }, [activeWorkspaceContext, activeWorkspaceRoot]);

  useEffect(() => {
    if (!workspacePickerOpen) return;
    workspacePathInputRef.current?.focus();
  }, [workspacePickerOpen]);

  useEffect(() => {
    if (!workspacePickerOpen || !workspacePathIsCompletable) {
      setWorkspaceCompletions([]);
      setWorkspaceParent(null);
      setWorkspaceCompletionIndex(-1);
      return;
    }

    let cancelled = false;
    const timer = window.setTimeout(() => {
      void api
        .completeWorkspacePath(workspacePath)
        .then(({ directories, parent }) => {
          if (cancelled) return;
          setWorkspaceCompletions(directories);
          setWorkspaceParent(parent);
          setWorkspaceCompletionIndex(-1);
        })
        .catch(() => {
          if (cancelled) return;
          setWorkspaceCompletions([]);
          setWorkspaceParent(null);
          setWorkspaceCompletionIndex(-1);
        });
    }, 150);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [workspacePath, workspacePathIsCompletable, workspacePickerOpen]);

  useEffect(() => {
    if (!workspacePickerOpen) return;

    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (target instanceof Node && !workspacePickerRef.current?.contains(target)) {
        setWorkspacePickerOpen(false);
        setWorkspacePickerError(null);
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setWorkspacePickerOpen(false);
        setWorkspacePickerError(null);
      }
    };

    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [workspacePickerOpen]);

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
      setCursorPosition(nextCursor);
      setCommandPaletteOpen(true);
      onChangeDraft(nextDraft);
      onFileMentionChange(nextDraft, nextCursor);
      requestAnimationFrame(() => {
        textarea.focus();
        textarea.setSelectionRange(nextCursor, nextCursor);
      });
    }
    onImagesPasted(files);
  };

  const selectWorkspace = async (path: string) => {
    const candidate = path.trim();
    if (!candidate || workspaceLoading) return;

    setWorkspaceLoading(true);
    setWorkspacePickerError(null);
    try {
      const context = await api.getWorkspaceContext(candidate);
      setSelectedWorkspaceRoot(context.workspace_root);
      setWorkspaceContext(context);
      setWorkspacePath(context.workspace_root);
      setWorkspacePickerOpen(false);
    } catch (reason) {
      setWorkspacePickerError(
        reason instanceof Error ? reason.message : t("Failed to inspect directory"),
      );
    } finally {
      setWorkspaceLoading(false);
    }
  };

  const openWorkspacePicker = () => {
    setWorkspacePath(activeWorkspaceRoot);
    setWorkspaceParent(null);
    setWorkspaceCompletionIndex(-1);
    setWorkspacePickerError(null);
    setWorkspacePickerOpen(true);
  };

  return (
    <section className="welcome-page">
      <div className="welcome-heading">
        <div className="welcome-logo">t</div>
        <h1>tidev</h1>
        <p>{t("Your intelligent coding assistant")}</p>
      </div>
      <div ref={composerRef} className="welcome-composer-shell">
        <div ref={workspacePickerRef} className="welcome-workspace-context">
          <Button
            className="welcome-workspace-button"
            type="button"
            aria-expanded={workspacePickerOpen}
            aria-haspopup="dialog"
            title={activeWorkspaceRoot}
            onClick={openWorkspacePicker}
            variant="secondary"
            size="sm"
            leadingIcon={<Folder size={15} strokeWidth={1.8} />}
            trailingIcon={<ChevronDown size={13} strokeWidth={1.8} />}
          >
            {workspaceLabel}
          </Button>
          {activeWorkspaceContext?.git_branch ? (
            <span className="welcome-workspace-branch" title={activeWorkspaceContext.git_branch}>
              <GitBranch size={15} strokeWidth={1.8} />
              <span>{activeWorkspaceContext.git_branch}</span>
            </span>
          ) : null}
          {workspacePickerOpen ? (
            <div
              className="welcome-workspace-picker"
              role="dialog"
              aria-label={t("Select directory")}
            >
              <form
                className="welcome-workspace-path-form"
                onSubmit={(event) => {
                  event.preventDefault();
                  const selected = workspaceCompletions[workspaceCompletionIndex];
                  void selectWorkspace(selected ?? workspacePath);
                }}
              >
                <Input
                  ref={workspacePathInputRef}
                  value={workspacePath}
                  onChange={(event) => {
                    setWorkspacePath(event.target.value);
                    setWorkspaceParent(null);
                    setWorkspaceCompletionIndex(-1);
                    setWorkspacePickerError(null);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Escape") {
                      event.preventDefault();
                      setWorkspacePickerOpen(false);
                      setWorkspacePickerError(null);
                      return;
                    }
                    if (workspaceCompletions.length === 0) return;
                    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                      event.preventDefault();
                      setWorkspaceCompletionIndex((current) => {
                        const delta = event.key === "ArrowDown" ? 1 : -1;
                        return (
                          (current + delta + workspaceCompletions.length) %
                          workspaceCompletions.length
                        );
                      });
                      return;
                    }
                    if (event.key === "Tab") {
                      const selected = workspaceCompletions[workspaceCompletionIndex];
                      if (!selected) return;
                      event.preventDefault();
                      setWorkspacePath(selected);
                      setWorkspaceCompletionIndex(-1);
                    }
                  }}
                  aria-label={t("Enter a path")}
                  autoComplete="off"
                  placeholder={t("Enter a path")}
                  spellCheck={false}
                />
                <Button
                  type="submit"
                  variant="primary"
                  size="sm"
                  loading={workspaceLoading}
                  disabled={!workspacePath.trim()}
                  className="welcome-workspace-submit"
                >
                  {t("Use directory")}
                </Button>
              </form>
              {workspacePickerError ? (
                <p className="welcome-workspace-error">{workspacePickerError}</p>
              ) : null}
              {workspaceParent ? (
                <div className="welcome-workspace-options">
                  <Button
                    type="button"
                    className="welcome-workspace-option"
                    title={workspaceParent}
                    onClick={() => {
                      setWorkspacePath(
                        workspaceParent === "/" ? workspaceParent : `${workspaceParent}/`,
                      );
                      setWorkspaceCompletionIndex(-1);
                    }}
                    variant="ghost"
                    size="sm"
                    leadingIcon={<Folder size={15} strokeWidth={1.8} />}
                  >
                    ..
                  </Button>
                </div>
              ) : null}
              {workspaceCompletions.length > 0 ? (
                <div className="welcome-workspace-options" role="listbox">
                  {workspaceCompletions.map((directory, index) => (
                    <Button
                      key={directory}
                      type="button"
                      className={
                        index === workspaceCompletionIndex
                          ? "welcome-workspace-option selected"
                          : "welcome-workspace-option"
                      }
                      role="option"
                      aria-selected={index === workspaceCompletionIndex}
                      onMouseEnter={() => setWorkspaceCompletionIndex(index)}
                      onClick={() => void selectWorkspace(directory)}
                      variant="ghost"
                      size="sm"
                      leadingIcon={<Folder size={15} strokeWidth={1.8} />}
                    >
                      {directory}
                    </Button>
                  ))}
                </div>
              ) : workspacePathIsCompletable && !workspaceParent ? (
                <p className="welcome-workspace-empty">{t("No matching directories.")}</p>
              ) : null}
              {recentDirectories.length > 0 ? (
                <div className="welcome-workspace-recent">
                  <p>{t("Recent directories")}</p>
                  <div className="welcome-workspace-options">
                    {recentDirectories.map((directory) => (
                      <Button
                        key={directory}
                        type="button"
                        className="welcome-workspace-option"
                        onClick={() => void selectWorkspace(directory)}
                        variant="ghost"
                        size="sm"
                        leadingIcon={<Folder size={15} strokeWidth={1.8} />}
                      >
                        {directory}
                      </Button>
                    ))}
                  </div>
                </div>
              ) : null}
            </div>
          ) : null}
        </div>
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
            <ImageAttachmentStrip
              images={pendingImages}
              onRemove={onRemoveImage}
              disabled={loading || sending}
            />
            <Textarea
              ref={textareaRef}
              value={draft}
              onPaste={handlePaste}
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
                compositionJustCommittedRef.current = false;
                if (compositionEndTimerRef.current) clearTimeout(compositionEndTimerRef.current);
              }}
              onCompositionEnd={() => {
                compositionRef.current = false;
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
                  !compositionRef.current &&
                  !compositionJustCommittedRef.current &&
                  ((enterToSend && !event.shiftKey) ||
                    (!enterToSend && (event.ctrlKey || event.metaKey)))
                ) {
                  event.preventDefault();
                  onSubmit(activeWorkspaceRoot);
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
            </div>
            <IconButton
              label={t("Start conversation")}
              size="md"
              variant="primary"
              className="send-button"
              disabled={
                (!draft.trim() && pendingImages.length === 0) ||
                !activeWorkspaceRoot ||
                loading ||
                sending
              }
              onClick={() => onSubmit(activeWorkspaceRoot)}
              title={t("Start conversation")}
            >
              {sending ? <LoaderCircle className="spin" size={17} /> : <Send size={17} />}
            </IconButton>
          </div>
        </div>
      </div>
      {error ? <div className="error-banner welcome-error">{error}</div> : null}
    </section>
  );
}
