import { useEffect, useState } from "react";
import { MessageSquare, Search, Sparkles, Wrench } from "lucide-react";
import { useTranslation } from "react-i18next";

import { api } from "../../api/client";
import type { MessageRecord } from "../../types/api";
import type { ToolCallEntry } from "../../utils/round";
import { ActivityRipple } from "./ActivityRipple";
import { MarkdownRenderer } from "./MarkdownRenderer";

function parseTaskArgs(entry: ToolCallEntry): {
  agent: string;
  description: string;
  prompt: string;
} {
  try {
    const args = JSON.parse(entry.arguments) as Record<string, unknown>;
    return {
      agent: typeof args.subagent_type === "string" ? args.subagent_type : "subagent",
      description: typeof args.description === "string" ? args.description : "",
      prompt: typeof args.prompt === "string" ? args.prompt : "",
    };
  } catch {
    return { agent: "subagent", description: "", prompt: "" };
  }
}

function agentIcon(agent: string) {
  const normalized = agent.toLowerCase();
  if (normalized === "explorer") return Search;
  if (normalized === "oracle") return Sparkles;
  return Wrench;
}

function agentLabel(agent: string, t: (key: string) => string) {
  return agent || t("Subagent");
}

function statusLabel(
  status: string | undefined,
  t: (key: string, options?: Record<string, unknown>) => string,
) {
  if (!status) return "";
  const match = status.match(/^Started (Explorer|Librarian|Oracle|Fixer) subagent$/i);
  if (!match) return status;
  return t("Started {{agent}} subagent", { agent: match[1] });
}

function ChildMessages({ records }: { records: MessageRecord[] }) {
  return (
    <div className="subagent-messages">
      {records.map(({ message }) => {
        if (message.role !== "assistant" || (!message.content && !message.reasoning)) return null;
        return (
          <div className="subagent-message" key={message.id}>
            {message.reasoning ? (
              <div className="subagent-reasoning">{message.reasoning}</div>
            ) : null}
            {message.content ? <MarkdownRenderer content={message.content} /> : null}
          </div>
        );
      })}
    </div>
  );
}

export function SubagentCard({ entry }: { entry: ToolCallEntry }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const [records, setRecords] = useState<MessageRecord[] | null>(null);
  const [loading, setLoading] = useState(false);
  const { agent, description, prompt } = parseTaskArgs(entry);
  const Icon = agentIcon(agent);
  const running = entry.status === "pending" || entry.status === "running";
  const displayedStatus = statusLabel(entry.subagentStatus, t);

  useEffect(() => {
    if (!expanded || !entry.childSessionId || records || loading) return;
    setLoading(true);
    void api
      .listMessages(entry.childSessionId)
      .then((response) => setRecords(response.messages))
      .catch((error) => console.error("Failed to load subagent messages", error))
      .finally(() => setLoading(false));
  }, [entry.childSessionId, expanded, loading, records]);

  return (
    <div className="tool-renderer subagent-renderer">
      <button
        className="tool-renderer-header"
        onClick={() => setExpanded((value) => !value)}
        aria-expanded={expanded}
      >
        <ActivityRipple active={running} row label={t("Subagent is running")}>
          <Icon size={14} />
          <span className="tool-renderer-title">
            <strong>{agentLabel(agent, t)}</strong>
            <span>{displayedStatus || description || t("Delegated task")}</span>
          </span>
        </ActivityRipple>
      </button>
      <div className={`tool-renderer-body-shell${expanded ? " expanded" : ""}`}>
        <div className="tool-renderer-body">
          {prompt ? (
            <div className="subagent-prompt">
              <span>{t("Task prompt")}</span>
              <p>{prompt}</p>
            </div>
          ) : null}
          {entry.childSessionId ? (
            <div className="subagent-session-label">
              <MessageSquare size={13} />
              <span>{t("Sub-session")}</span>
            </div>
          ) : null}
          {entry.subagentReasoningDelta ? (
            <div className="subagent-live-reasoning">{entry.subagentReasoningDelta}</div>
          ) : null}
          {entry.subagentContentDelta ? (
            <MarkdownRenderer content={entry.subagentContentDelta} />
          ) : null}
          {loading ? <div className="tool-loading">{t("Loading sub-session…")}</div> : null}
          {records ? <ChildMessages records={records} /> : null}
          {!records && entry.result?.output ? (
            <MarkdownRenderer content={entry.result.output} />
          ) : null}
        </div>
      </div>
    </div>
  );
}
