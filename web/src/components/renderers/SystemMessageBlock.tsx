import { FileText } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { api } from "../../api/client";
import type { Message } from "../../types/api";
import { parseInstructionMessage, type InstructionMessageDetails } from "../../utils/round";
import { ExpandableBody } from "../ui/ExpandableBody";
import { MarkdownRenderer } from "./MarkdownRenderer";

const COMPACTION_LABEL = "Compaction";

interface InstructionMessageProps {
  message: Message;
  content?: string;
  sessionId?: string;
  expanded?: boolean;
  onExpandedChange?: (expanded: boolean) => void;
}

export function InstructionMessage({
  message,
  content,
  sessionId,
  expanded: controlledExpanded,
  onExpandedChange,
}: InstructionMessageProps) {
  const details = parseInstructionMessage(message.content);
  if (!details) return null;

  return (
    <InstructionMessageContent
      message={message}
      content={content}
      sessionId={sessionId}
      controlledExpanded={controlledExpanded}
      onExpandedChange={onExpandedChange}
      details={details}
    />
  );
}

function InstructionMessageContent({
  message,
  content,
  sessionId,
  controlledExpanded,
  onExpandedChange,
  details,
}: InstructionMessageProps & {
  details: InstructionMessageDetails;
  controlledExpanded?: boolean;
}) {
  const { t } = useTranslation();
  const [localExpanded, setLocalExpanded] = useState(false);
  const [loadedContent, setLoadedContent] = useState<string | null>(null);
  const [loadState, setLoadState] = useState<"idle" | "loading" | "loaded" | "error">("idle");
  const expanded = controlledExpanded ?? localExpanded;

  const sourcePaths = details.sources
    .split(",")
    .map((source) => source.trim())
    .filter(Boolean);
  const sourceKey = sourcePaths.join("\u0000");
  const resolvedContent = content ?? loadedContent;
  const bodyId = `instruction-content-${message.id.replace(/[^a-zA-Z0-9_-]/g, "-")}`;

  useEffect(() => {
    setLoadedContent(null);
    setLoadState("idle");
  }, [content, message.id, sourceKey]);

  useEffect(() => {
    if (!expanded || resolvedContent !== null || loadState !== "idle") return;
    if (!sessionId) {
      setLoadState("error");
      return;
    }

    let cancelled = false;
    setLoadState("loading");
    api
      .listMessages(sessionId)
      .then((response) => {
        const blocks = response.messages.flatMap(
          ({ message: persistedMessage }) =>
            persistedMessage.content.match(/<system-reminder>[\s\S]*?<\/system-reminder>/g) ?? [],
        );
        const matchingBlocks = blocks.filter((block) => {
          const blockSources = block.match(/^Instructions from:\s*(.+)$/gm) ?? [];
          return blockSources.some((line) => {
            const actualSource = line
              .replace(/^Instructions from:\s*/, "")
              .trim()
              .replaceAll("\\", "/");
            return sourceKey.split("\u0000").some((source) => {
              const expectedSource = source.replaceAll("\\", "/");
              return (
                actualSource === expectedSource ||
                actualSource.endsWith(`/${expectedSource}`) ||
                expectedSource.endsWith(`/${actualSource}`)
              );
            });
          });
        });
        const parts =
          matchingBlocks.length > 0 ? matchingBlocks : blocks.length === 1 ? blocks : [];
        if (cancelled) return;
        if (parts.length > 0) {
          setLoadedContent(parts.join("\n\n"));
          setLoadState("loaded");
        } else {
          setLoadState("error");
        }
      })
      .catch(() => {
        if (!cancelled) setLoadState("error");
      });

    return () => {
      cancelled = true;
    };
  }, [expanded, loadState, resolvedContent, sessionId, sourceKey]);

  function toggleExpanded() {
    const next = !expanded;
    if (controlledExpanded === undefined) setLocalExpanded(next);
    onExpandedChange?.(next);
  }

  return (
    <div className="tool-renderer instruction-renderer">
      <button
        type="button"
        className="tool-renderer-header instruction-message"
        onClick={toggleExpanded}
        aria-expanded={expanded}
        aria-controls={bodyId}
      >
        <FileText size={14} />
        <span className="tool-renderer-title">
          <strong>
            {details.count === null
              ? t("Loaded instructions from")
              : t("Loaded {{count}} instruction files", { count: details.count })}
          </strong>
          <code>{details.sources}</code>
        </span>
      </button>
      <ExpandableBody expanded={expanded} className="tool-renderer-body-shell">
        <div id={bodyId} className="tool-renderer-body instruction-body">
          {resolvedContent !== null ? (
            <pre className="tool-raw-output instruction-content">{resolvedContent}</pre>
          ) : loadState === "loading" ? (
            <span className="tool-empty-output instruction-body-status">
              {t("Loading instruction content")}
            </span>
          ) : (
            <span className="tool-empty-output instruction-body-status">
              {t("Instruction content unavailable")}
            </span>
          )}
        </div>
      </ExpandableBody>
    </div>
  );
}

export function SystemMessageBlock({
  message,
  sessionId,
}: {
  message: Message;
  sessionId?: string;
}) {
  const { t } = useTranslation();

  if (parseInstructionMessage(message.content)) {
    return (
      <article className="system-message-block">
        <InstructionMessage message={message} sessionId={sessionId} />
      </article>
    );
  }

  if (message.content.startsWith(COMPACTION_LABEL)) {
    const summary = message.content.split("\n\n").slice(1).join("\n\n").trim();
    return (
      <article className="system-message-block">
        <div className="system-divider">
          <span />
          <strong>{t(COMPACTION_LABEL)}</strong>
          <span />
        </div>
        {summary ? <MarkdownRenderer content={summary} /> : null}
      </article>
    );
  }

  return (
    <article className="system-message-block">
      <MarkdownRenderer content={message.content} />
    </article>
  );
}
