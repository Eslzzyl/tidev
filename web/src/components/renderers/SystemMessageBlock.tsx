import { FileText } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { Message } from "../../types/api";
import { parseInstructionMessage } from "../../utils/round";
import { MarkdownRenderer } from "./MarkdownRenderer";

const COMPACTION_LABEL = "Compaction";

export function InstructionMessage({ message }: { message: Message }) {
  const { t } = useTranslation();
  const details = parseInstructionMessage(message.content);
  if (!details) return null;

  return (
    <div className="instruction-message">
      <FileText size={14} />
      <span>
        {details.count === null
          ? t("Loaded instructions from")
          : t("Loaded {{count}} instruction files", { count: details.count })}
      </span>
      <code>{details.sources}</code>
    </div>
  );
}

export function SystemMessageBlock({ message }: { message: Message }) {
  const { t } = useTranslation();

  if (parseInstructionMessage(message.content)) {
    return (
      <article className="system-message-block">
        <InstructionMessage message={message} />
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
