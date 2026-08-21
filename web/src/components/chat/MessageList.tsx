import { useMemo, useRef } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Check, Sparkles, X } from "lucide-react";

import type { ApprovedTool, FrontendRequest, MessageRecord, ToolCall } from "../../types/api";
import type { StreamMessage } from "../../types/chat";
import {
  buildRounds,
  type Round,
  type ShellBlock,
  type SystemMessageBlock,
} from "../../utils/round";
import { formatTime, getDuration, stripSystemReminderTags } from "../../utils/format";

export interface MessageListProps {
  messages: MessageRecord[];
  streams: StreamMessage[];
  onRevert?: (messageId: string) => void;
  onFork?: (messageId: string) => void;
}

export function MessageList({ messages, streams, onRevert, onFork }: MessageListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const rounds = useMemo(() => buildRounds(messages), [messages]);
  type Row =
    | { type: "round"; key: string; round: Round }
    | { type: "system"; key: string; block: SystemMessageBlock }
    | { type: "shell"; key: string; block: ShellBlock }
    | { type: "stream"; key: string; stream: StreamMessage };

  const rows = useMemo<Row[]>(() => {
    const base: Row[] = rounds.map((item) => {
      if ((item as ShellBlock).kind === "shell") {
        const block = item as ShellBlock;
        return { type: "shell", key: block.id, block };
      }
      if ((item as SystemMessageBlock).kind === "system") {
        const block = item as SystemMessageBlock;
        return { type: "system", key: block.id, block };
      }
      const round = item as Round;
      return { type: "round", key: round.id, round };
    });
    return base.concat(streams.map((stream) => ({ type: "stream", key: stream.key, stream })));
  }, [rounds, streams]);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 160,
    overscan: 8,
    getItemKey: (index) => rows[index]?.key ?? index,
  });

  if (rows.length === 0) {
    return (
      <div className="welcome-state">
        <div className="welcome-icon">
          <Sparkles size={21} />
        </div>
        <h2>What are we building?</h2>
        <p>
          Start a conversation with the local tidev runtime. Your messages and streamed responses
          are persisted in SQLite.
        </p>
      </div>
    );
  }

  return (
    <div className="message-scroll" ref={scrollRef}>
      <div className="message-virtual-canvas" style={{ height: `${virtualizer.getTotalSize()}px` }}>
        {virtualizer.getVirtualItems().map((item) => {
          const row = rows[item.index];
          return (
            <div
              className="message-row"
              data-index={item.index}
              key={item.key}
              ref={virtualizer.measureElement}
              style={{ transform: `translateY(${item.start}px)` }}
            >
              {row.type === "round" ? (
                <RoundView round={row.round} onRevert={onRevert} onFork={onFork} />
              ) : row.type === "shell" ? (
                <ShellBlockView block={row.block} />
              ) : row.type === "system" ? (
                <SystemBlockView block={row.block} />
              ) : (
                <StreamBubble stream={row.stream} />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function RoundView({
  round,
  onRevert,
  onFork,
}: {
  round: Round;
  onRevert?: (messageId: string) => void;
  onFork?: (messageId: string) => void;
}) {
  const userTime = round.userMessage.created_at ? formatTime(round.userMessage.created_at) : "";
  const duration = round.completedAt
    ? getDuration(round.userMessage.created_at ?? "", round.completedAt)
    : null;
  const footerParts: string[] = [];
  if (round.modelName) footerParts.push(round.modelName);
  if (duration) footerParts.push(duration);
  if (round.completedAt) footerParts.push(formatTime(round.completedAt));
  else if (round.status === "streaming" && userTime) footerParts.push(userTime);

  const hasAssistant = round.segments.length > 0 || round.status !== "user_only";
  return (
    <div className="round-group">
      <article className="message-card user">
        <div className="message-layout">
          <span className="avatar user-avatar">U</span>
          <div className="message-column">
            <div className="message-meta">
              <span>You</span>
              {userTime ? <time>{userTime}</time> : null}
              <span className="message-actions-inline">
                {onRevert ? (
                  <button
                    className="inline-action"
                    onClick={() => onRevert(round.userMessage.id)}
                    title="Revert to this message (undo later messages)"
                  >
                    Undo
                  </button>
                ) : null}
                {onFork ? (
                  <button
                    className="inline-action"
                    onClick={() => onFork(round.userMessage.id)}
                    title="Fork conversation from this message"
                  >
                    Fork
                  </button>
                ) : null}
              </span>
            </div>
            <div className="message-content">
              <p className="plain-content">{stripSystemReminderTags(round.userMessage.content)}</p>
            </div>
          </div>
        </div>
      </article>
      {hasAssistant ? (
        <article className="message-card assistant">
          <div className="message-layout">
            <span className="avatar">A</span>
            <div className="message-column">
              <div className="message-meta">
                <span>Assistant</span>
                {round.status === "streaming" ? (
                  <span className="streaming-label">streaming</span>
                ) : null}
              </div>
              <div className="message-content">
                {round.segments.map((segment, index) => {
                  if (segment.type === "reasoning" && segment.content) {
                    return (
                      <details
                        key={index}
                        className="reasoning"
                        open={round.status === "streaming"}
                      >
                        <summary>Reasoning</summary>
                        <div>{segment.content}</div>
                      </details>
                    );
                  }
                  if (segment.type === "text" && segment.content) {
                    return (
                      <ReactMarkdown key={index} remarkPlugins={[remarkGfm]}>
                        {stripSystemReminderTags(segment.content)}
                      </ReactMarkdown>
                    );
                  }
                  if (segment.type === "tool_call") {
                    const entry = round.toolCallMap[segment.toolCallId];
                    if (!entry) return null;
                    return <ToolCallEntryView key={index} entry={entry} />;
                  }
                  return null;
                })}
                {round.status === "streaming" && round.segments.length === 0 ? (
                  <span className="cursor-block" />
                ) : null}
                {round.status === "complete" && footerParts.length ? (
                  <div className="round-footer">{footerParts.join(" · ")}</div>
                ) : null}
              </div>
            </div>
          </div>
        </article>
      ) : null}
    </div>
  );
}

function SystemBlockView({ block }: { block: SystemMessageBlock }) {
  return (
    <article className="message-card system">
      <div className="message-layout">
        <span className="avatar">S</span>
        <div className="message-column">
          <div className="message-meta">
            <span>System</span>
            {block.message.created_at ? <time>{formatTime(block.message.created_at)}</time> : null}
          </div>
          <div className="message-content">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{block.message.content}</ReactMarkdown>
          </div>
        </div>
      </div>
    </article>
  );
}

function ShellBlockView({ block }: { block: ShellBlock }) {
  const exit = block.exitCode;
  return (
    <article className="message-card shell">
      <div className="message-layout">
        <span className="avatar">S</span>
        <div className="message-column">
          <div className="message-meta">
            <span>Shell</span>
            {exit !== null && exit !== undefined ? (
              <span className={exit === 0 ? "shell-exit ok" : "shell-exit fail"}>exit {exit}</span>
            ) : null}
          </div>
          <div className="message-content">
            <pre className="shell-command">{block.command.content}</pre>
            <pre className="shell-output">{block.output.content}</pre>
          </div>
        </div>
      </div>
    </article>
  );
}

function ToolCallEntryView({ entry }: { entry: import("../../utils/round").ToolCallEntry }) {
  const summary = (() => {
    try {
      const args = JSON.parse(entry.arguments);
      if (entry.name === "read" || entry.name === "write" || entry.name === "edit") {
        return args.file_path || args.path || "";
      }
      if (entry.name === "bash") return args.command || "";
      if (entry.name === "grep") return args.pattern ? `"${args.pattern}"` : "";
      if (entry.name === "glob") return args.pattern || "";
      return entry.arguments.slice(0, 80);
    } catch {
      return entry.arguments.slice(0, 80);
    }
  })();
  return (
    <div className="tool-entry">
      <div className="tool-entry-header">
        <strong>{entry.name}</strong>
        <code className="tool-args">{summary}</code>
        {!entry.resultComplete ? <span className="tool-running">running…</span> : null}
      </div>
      {entry.result ? (
        <pre className="tool-result">{entry.result.output.slice(0, 2000)}</pre>
      ) : null}
    </div>
  );
}

function StreamBubble({ stream }: { stream: StreamMessage }) {
  return (
    <article className="message-card assistant streaming-card">
      <div className="message-layout">
        <span className="avatar">A</span>
        <div className="message-column">
          <div className="message-meta">
            <span>Assistant</span>
            <span className="streaming-label">streaming</span>
          </div>
          <div className="message-content">
            {stream.reasoning ? (
              <details className="reasoning" open>
                <summary>Reasoning</summary>
                <div>{stream.reasoning}</div>
              </details>
            ) : null}
            {stream.content ? (
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{stream.content}</ReactMarkdown>
            ) : (
              <span className="cursor-block" />
            )}
            {stream.toolCalls.length ? <ToolCallList calls={stream.toolCalls} /> : null}
            {stream.status === "failed" ? (
              <div className="stream-error">{stream.error ?? "The turn failed."}</div>
            ) : null}
          </div>
        </div>
      </div>
    </article>
  );
}

function ToolCallList({ calls }: { calls: ToolCall[] }) {
  return (
    <div className="tool-list">
      {calls.map((call) => (
        <div className="tool-chip" key={call.id}>
          <span>{call.name}</span>
          <code>{call.arguments}</code>
        </div>
      ))}
    </div>
  );
}

function makeRejectedTool(tool: ToolCall): ApprovedTool {
  return {
    tool_call: tool,
    rejection: {
      output: "The user rejected this tool call.",
      attachments: [],
      metadata: {},
    },
    child_session_id: null,
    allow_outside: false,
    sensitive_file_approved: false,
    user_reason: "Rejected in Web UI",
  };
}

function makeApprovedTool(tool: ToolCall): ApprovedTool {
  return {
    tool_call: tool,
    rejection: null,
    child_session_id: null,
    allow_outside: true,
    sensitive_file_approved: true,
    user_reason: null,
  };
}

export function ApprovalCard({
  request,
  onRespond,
}: {
  request: FrontendRequest;
  onRespond: (tools: ApprovedTool[]) => void;
}) {
  const tools = request.kind.ToolApproval ?? [];
  return (
    <div className="approval-card">
      <div className="approval-heading">
        <span>
          <Sparkles size={16} /> Approval required
        </span>
        <span className="approval-session">{request.session_id.slice(0, 8)}</span>
      </div>
      <p>
        tidev is waiting for permission to run{" "}
        {tools.length === 1 ? "a tool" : `${tools.length} tools`}.
      </p>
      <div className="approval-tools">
        {tools.map((item) => (
          <div className="approval-tool" key={item.tool_call.id}>
            <strong>{item.tool_call.name}</strong>
            <code>{item.tool_call.arguments}</code>
          </div>
        ))}
      </div>
      <div className="approval-actions">
        <button
          className="secondary-button"
          onClick={() => onRespond(tools.map((item) => makeRejectedTool(item.tool_call)))}
        >
          <X size={15} />
          Reject
        </button>
        <button
          className="primary-button"
          onClick={() => onRespond(tools.map((item) => makeApprovedTool(item.tool_call)))}
        >
          <Check size={15} />
          Allow
        </button>
      </div>
    </div>
  );
}
