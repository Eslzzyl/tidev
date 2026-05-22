import { Terminal, ChevronDown, Loader2 } from "lucide-react";
import { useState, useRef, useEffect } from "react";
import type { ShellBlock as ShellBlockType } from "../../types/round";

interface Props {
  block: ShellBlockType;
}

/**
 * Strip markdown code fences (``` ... ```) from shell output.
 * The backend wraps output in code blocks for consistency with the TUI,
 * but in the tool-card UI we show it as raw preformatted text.
 */
function stripCodeFences(text: string): string {
  // Remove leading ```lang? and trailing ```
  return text.replace(/^```\w*\n?/, "").replace(/\n```\s*$/, "");
}

export function ShellBlock({ block }: Props) {
  const { command, output, exitCode } = block;
  const isStreaming = output.streaming;
  const outputContent = output.content || "";
  const [expanded, setExpanded] = useState(true);
  const didAutoExpand = useRef(false);

  // Auto-expand when result arrives
  useEffect(() => {
    if (didAutoExpand.current) return;
    if (!isStreaming && outputContent) {
      didAutoExpand.current = true;
      setExpanded(true);
    }
  }, [isStreaming, outputContent]);

  const displayOutput = stripCodeFences(outputContent);
  const hasOutput = displayOutput.length > 0;
  const isComplete = !isStreaming && hasOutput;

  return (
    <div className="border-b border-neutral-100 dark:border-neutral-900">
      <div className="group flex gap-3 px-4 py-4">
        {/* Avatar: Terminal icon in violet circle */}
        <div className="flex-shrink-0">
          <div className="flex h-8 w-8 items-center justify-center rounded-full bg-violet-500 text-xs font-medium text-white">
            <Terminal className="h-4 w-4" />
          </div>
        </div>

        {/* Content area — matches assistant `max-w-[85%] flex-1` */}
        <div className="flex max-w-[85%] flex-1 flex-col items-start">
          {/* Label row */}
          <div className="mb-1 flex items-center gap-2">
            <span className="text-xs font-medium text-neutral-500 dark:text-neutral-400">
              Shell
            </span>
          </div>

          {/* Card */}
          <div className="w-full overflow-hidden rounded-lg border border-violet-200 bg-violet-50 dark:border-violet-800 dark:bg-violet-950/30">
        {/* Header (collapsed) */}
        <button
          onClick={() => setExpanded((v) => !v)}
          className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-black/5 dark:hover:bg-white/5"
        >
          <Terminal className="h-3.5 w-3.5 flex-shrink-0 text-violet-600 dark:text-violet-400" />

          <div className="flex flex-1 flex-col min-w-0">
            <span className="text-xs font-medium text-neutral-700 dark:text-neutral-300">
              Shell
            </span>
            <span className="truncate font-mono text-xs text-neutral-500 dark:text-neutral-400">
              {command.content}
            </span>
          </div>

          {/* Status */}
          <div className="flex items-center gap-2 flex-shrink-0">
            {isStreaming && (
              <>
                <Loader2 className="h-3.5 w-3.5 animate-spin text-neutral-400" />
                <span className="text-xs text-neutral-400">running...</span>
              </>
            )}
            {isComplete && exitCode === 0 && (
              <span className="text-xs text-green-600 dark:text-green-400">
                &#10003;
              </span>
            )}
            {isComplete && exitCode !== null && exitCode !== 0 && (
              <span className="text-xs text-red-600 dark:text-red-400">
                &#10007;
              </span>
            )}
            <ChevronDown
              className={`h-3.5 w-3.5 text-neutral-400 transition-transform ${expanded ? "rotate-180" : ""}`}
            />
          </div>
        </button>

        {/* Expanded content */}
        {expanded && (
          <>
            {isStreaming && (
              <div className="border-t border-inherit px-3 py-3">
                <div className="flex items-center gap-1.5 text-violet-500">
                  <span className="h-2 w-2 animate-pulse rounded-full bg-violet-500" />
                  <span className="text-xs">running...</span>
                </div>
              </div>
            )}

            {hasOutput && (
              <div className="border-t border-inherit">
                <div className="px-3 py-2 space-y-1">
                  {/* Command */}
                  <div className="overflow-x-auto whitespace-pre-wrap break-all rounded bg-black/5 px-3 py-1.5 font-mono text-xs leading-relaxed text-neutral-600 dark:bg-white/5 dark:text-neutral-400">
                    {command.content}
                  </div>

                  {/* Output */}
                  <pre className="overflow-x-auto whitespace-pre-wrap font-mono text-xs leading-relaxed text-neutral-700 dark:text-neutral-300">
                    {displayOutput}
                  </pre>
                </div>
              </div>
            )}

            {!hasOutput && !isStreaming && (
              <div className="border-t border-inherit px-3 py-2">
                <span className="text-xs text-neutral-500 dark:text-neutral-400">
                  {outputContent || "Command completed (no output)"}
                </span>
              </div>
            )}

            {/* Exit code footer */}
            {exitCode !== null && (
              <div className="border-t border-inherit bg-black/5 px-3 py-1 text-xs dark:bg-white/5">
                <span className="text-neutral-500 dark:text-neutral-400">
                  Exit code: {exitCode}
                  {exitCode === 0 ? (
                    <span className="text-green-600 dark:text-green-400">
                      {" "}&#10003;
                    </span>
                  ) : (
                    <span className="text-red-600 dark:text-red-400">
                      {" "}&#10007;
                    </span>
                  )}
                </span>
              </div>
            )}
          </>
        )}
      </div>{/* card */}
    </div>{/* content area */}
  </div>{/* flex row */}
</div>
  );
}
