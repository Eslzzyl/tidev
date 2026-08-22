import type { Message } from "../../types/api";
import { MarkdownRenderer } from "./MarkdownRenderer";

const COMPACTION_LABEL = "Compaction";

export function SystemMessageBlock({ message }: { message: Message }) {
  if (message.content.startsWith(COMPACTION_LABEL)) {
    const summary = message.content.split("\n\n").slice(1).join("\n\n").trim();
    return (
      <article className="system-message-block">
        <div className="system-divider">
          <span />
          <strong>{COMPACTION_LABEL}</strong>
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
