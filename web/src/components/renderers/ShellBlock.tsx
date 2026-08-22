import { useEffect, useRef, useState } from "react";
import { ChevronDown, Loader2, Terminal } from "lucide-react";

import type { ShellBlock as ShellBlockType } from "../../utils/round";

function stripCodeFences(text: string): string {
  return text.replace(/^```\w*\n?/, "").replace(/\n```\s*$/, "");
}

export function ShellBlock({ block }: { block: ShellBlockType }) {
  const [expanded, setExpanded] = useState(true);
  const autoExpanded = useRef(false);
  const outputContent = block.output.content || "";
  const isStreaming = block.output.streaming === true;
  const displayOutput = stripCodeFences(outputContent);
  const hasOutput = displayOutput.length > 0;

  useEffect(() => {
    if (autoExpanded.current || isStreaming || !hasOutput) return;
    autoExpanded.current = true;
    const frame = requestAnimationFrame(() => setExpanded(true));
    return () => cancelAnimationFrame(frame);
  }, [hasOutput, isStreaming]);

  return (
    <article className="shell-message-block">
      <div className="message-layout">
        <div className="message-column">
          <div className="message-meta">
            <span>Shell</span>
          </div>
          <div className="shell-renderer">
            <button className="shell-header" onClick={() => setExpanded((value) => !value)}>
              <Terminal size={14} />
              <span className="shell-heading">
                <strong>Shell</strong>
                <code>{block.command.content}</code>
              </span>
              <span className="shell-status">
                {isStreaming ? <Loader2 className="spin" size={14} /> : null}
                {!isStreaming && block.exitCode === 0 ? (
                  <span className="shell-success">✓</span>
                ) : null}
                {!isStreaming && block.exitCode !== null && block.exitCode !== 0 ? (
                  <span className="shell-failure">×</span>
                ) : null}
                <ChevronDown
                  className={expanded ? "thinking-chevron expanded" : "thinking-chevron"}
                  size={14}
                />
              </span>
            </button>
            {expanded ? (
              <div className="shell-body">
                {isStreaming ? (
                  <div className="shell-running">
                    <span />
                    running...
                  </div>
                ) : null}
                {hasOutput ? (
                  <>
                    <code className="shell-command">{block.command.content}</code>
                    <pre>{displayOutput}</pre>
                  </>
                ) : !isStreaming ? (
                  <span className="tool-empty-output">
                    {outputContent || "Command completed (no output)"}
                  </span>
                ) : null}
                {block.exitCode !== null ? (
                  <div className="shell-footer">
                    Exit code: {block.exitCode} {block.exitCode === 0 ? "✓" : "×"}
                  </div>
                ) : null}
              </div>
            ) : null}
          </div>
        </div>
      </div>
    </article>
  );
}
