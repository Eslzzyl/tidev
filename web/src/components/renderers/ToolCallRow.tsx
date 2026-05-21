import { useState, useRef, useEffect, memo } from "react";
import {
  Loader2,
  ChevronDown,
  FileText,
  Search,
  Files,
  FileEdit,
  FileDiff,
  Terminal,
  Sparkles,
  LayoutTemplate,
  ListTodo,
  Wrench,
  Clock,
} from "lucide-react";
import type { ToolCallEntry } from "../../types/round";
import { MarkdownRenderer } from "./MarkdownRenderer";
import { DiffRenderer } from "./DiffRenderer";
import { CodeLinesRenderer } from "./CodeLinesRenderer";
import { TodoRenderer } from "./TodoRenderer";
import { JsonTreeView } from "../ui/JsonTreeView";

interface Props {
  entry: ToolCallEntry;
}

function isReadOnlyTool(name: string): boolean {
  return ["read", "grep", "glob", "skill"].includes(name);
}

function isWriteTool(name: string): boolean {
  return ["write", "edit", "apply_patch"].includes(name);
}

function isBash(name: string): boolean {
  return name === "bash";
}

function isTodo(name: string): boolean {
  return name === "todowrite";
}

function isWebTool(name: string): boolean {
  return ["websearch", "webfetch"].includes(name);
}

function getToolIcon(
  name: string,
): React.ComponentType<{ className?: string }> {
  switch (name) {
    case "read":
      return FileText;
    case "grep":
      return Search;
    case "glob":
      return Files;
    case "write":
    case "edit":
      return FileEdit;
    case "apply_patch":
      return FileDiff;
    case "bash":
      return Terminal;
    case "skill":
      return Sparkles;
    case "task":
      return LayoutTemplate;
    case "todowrite":
      return ListTodo;
    default:
      return Wrench;
  }
}

function getToolColor(name: string): string {
  if (isReadOnlyTool(name)) return "text-blue-600 dark:text-blue-400";
  if (isWriteTool(name)) return "text-emerald-600 dark:text-emerald-400";
  if (isBash(name)) return "text-violet-600 dark:text-violet-400";
  if (name === "task") return "text-amber-600 dark:text-amber-400";
  if (isTodo(name)) return "text-rose-600 dark:text-rose-400";
  if (isWebTool(name)) return "text-sky-600 dark:text-sky-400";
  return "text-neutral-600 dark:text-neutral-400";
}

function getToolBg(name: string): string {
  if (isReadOnlyTool(name))
    return "bg-blue-50 dark:bg-blue-950/30 border-blue-200 dark:border-blue-800";
  if (isWriteTool(name)) return "border-neutral-200 dark:border-neutral-700";
  if (isBash(name))
    return "bg-violet-50 dark:bg-violet-950/30 border-violet-200 dark:border-violet-800";
  if (name === "task")
    return "bg-amber-50 dark:bg-amber-950/30 border-amber-200 dark:border-amber-800";
  if (isTodo(name))
    return "bg-rose-50 dark:bg-rose-950/30 border-rose-200 dark:border-rose-800";
  if (isWebTool(name))
    return "bg-sky-50 dark:bg-sky-950/30 border-sky-200 dark:border-sky-800";
  return "bg-neutral-50 dark:bg-neutral-800 border-neutral-200 dark:border-neutral-700";
}

function getToolLabel(name: string): string {
  switch (name) {
    case "read":
      return "Read";
    case "grep":
      return "Search";
    case "glob":
      return "Find";
    case "skill":
      return "Loaded skill";
    case "bash":
      return "bash";
    case "todowrite":
      return "Todos";
    case "websearch":
      return "Web Search";
    case "webfetch":
      return "Web Fetch";
    default:
      return name;
  }
}

function summarizeArguments(name: string, entry: ToolCallEntry): string {
  try {
    const args = JSON.parse(entry.arguments);
    switch (name) {
      case "read":
      case "write":
      case "edit": {
        return args.file_path || "(unknown)";
      }
      case "grep": {
        const pattern = args.pattern || "";
        const path = args.path || ".";
        return pattern ? `"${pattern}" in ${path}` : path;
      }
      case "glob": {
        const pattern = args.pattern || "*";
        return `${pattern}`;
      }
      case "bash": {
        return args.command || "(no command)";
      }
      case "apply_patch": {
        return args.path || "(unknown)";
      }
      case "skill": {
        return args.name || "(unknown)";
      }
      case "task": {
        return args.description || "(no description)";
      }
      case "todowrite": {
        const todos = args.todos;
        if (Array.isArray(todos)) {
          return `${todos.length} item(s)`;
        }
        return "(todos)";
      }
      case "websearch": {
        let summary = args.query || "(no query)";
        if (args.offset !== undefined) {
          summary += ` [offset=${args.offset}]`;
        }
        return summary;
      }
      case "webfetch": {
        let summary = args.url || "(no url)";
        if (args.offset !== undefined || args.limit !== undefined) {
          summary += ` [line ${args.offset ?? 1}`;
          if (args.limit !== undefined) summary += `, limit=${args.limit}`;
          summary += `]`;
        }
        return summary;
      }
      default:
        return entry.arguments.length > 60
          ? entry.arguments.slice(0, 60) + "..."
          : entry.arguments;
    }
  } catch {
    return entry.arguments || "...";
  }
}

function getResultSummary(entry: ToolCallEntry): string {
  if (!entry.result) return " ...";
  const output = entry.result.output;
  if (entry.result.isError) return " failed";

  const name = entry.name;
  const canonical = ["grep", "glob", "skill"].includes(name) ? name : "";

  switch (canonical) {
    case "grep":
    case "glob": {
      const firstLine = output.split("\n")[0] || "";
      let count: number;
      if (firstLine.startsWith("No files found")) {
        count = 0;
      } else if (firstLine.startsWith("Found ")) {
        // "Found 42 matches" or "Found 10 files"
        const numStr = firstLine.slice(6).split(" ")[0];
        count = parseInt(numStr, 10) || 0;
      } else {
        // Fallback: count non-empty lines
        count = output.split("\n").filter((l) => l.trim()).length;
      }
      if (count === 0) return " no match";
      if (count === 1) return " 1 match";
      return ` ${count} matches`;
    }
    default: {
      const firstLine = output.split("\n")[0] || "";
      if (firstLine.length > 80) return ` ${firstLine.slice(0, 80)}...`;
      return firstLine ? ` ${firstLine}` : " (empty)";
    }
  }
}

function getCollapsedLabel(entry: ToolCallEntry): string {
  const name = entry.name;
  if (isReadOnlyTool(name)) {
    return `${getToolLabel(name)} ${summarizeArguments(name, entry)}${entry.result ? getResultSummary(entry) : " ..."}`;
  }
  if (isWriteTool(name)) {
    return `${name === "apply_patch" ? "Apply patch" : name === "edit" ? "Edit" : "Write"} ${summarizeArguments(name, entry)}`;
  }
  if (isTodo(name)) {
    return `${getToolLabel(name)} ${summarizeArguments(name, entry)}`;
  }
  return `${name} ${summarizeArguments(name, entry)}`;
}

function getExitCode(entry: ToolCallEntry): number | null {
  return entry.result?.exitCode ?? null;
}

/**
 * Get the bash command from parsed arguments, for display purposes.
 */
function getBashCommand(entry: ToolCallEntry): string {
  try {
    const args = JSON.parse(entry.arguments);
    return args.command || "";
  } catch {
    return "";
  }
}

/**
 * Get the bash description from parsed arguments, for display purposes.
 */
function getBashDescription(entry: ToolCallEntry): string | null {
  try {
    const args = JSON.parse(entry.arguments);
    return args.description || null;
  } catch {
    return null;
  }
}

/** Format milliseconds into a human-readable duration string */
function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.floor(ms / 60000)}m ${Math.floor((ms % 60000) / 1000)}s`;
}

/** Try to parse output as JSON for tree view display */
function tryParseJson(output: string): unknown | null {
  try {
    const parsed = JSON.parse(output);
    if (typeof parsed === "object" && parsed !== null) return parsed;
    return null;
  } catch {
    return null;
  }
}

/** Check if output looks like JSON (starts with { or [) */
function looksLikeJson(output: string): boolean {
  const trimmed = output.trim();
  return trimmed.startsWith("{") || trimmed.startsWith("[");
}

export const ToolCallRow = memo(function ToolCallRow({ entry }: Props) {
  const [expanded, setExpanded] = useState(false);
  const [elapsedMs, setElapsedMs] = useState(0);
  const didAutoExpand = useRef(false);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const startTimeRef = useRef<number | null>(null);

  // Determine auto-expand conditions
  const isEmptyResult =
    entry.result &&
    entry.resultComplete &&
    (!entry.result.output ||
      entry.result.output.trim() === "" ||
      entry.result.output.trim() === "Done");

  const hasBashOutput =
    isBash(entry.name) && entry.resultComplete && !isEmptyResult;
  const hasDiff = entry.result && entry.result.diff;

  // Auto-expand once when result arrives
  useEffect(() => {
    if (didAutoExpand.current) return;
    if (hasBashOutput || (hasDiff && entry.resultComplete)) {
      didAutoExpand.current = true;
      setExpanded(true);
    }
  }, [hasBashOutput, hasDiff, entry.resultComplete]);

  // Live elapsed timer for running tool calls
  useEffect(() => {
    if (!entry.resultComplete && entry.argumentsComplete) {
      // Tool is running — track elapsed time
      startTimeRef.current = Date.now();
      timerRef.current = setInterval(() => {
        setElapsedMs(Date.now() - (startTimeRef.current ?? Date.now()));
      }, 100);
    } else if (entry.resultComplete) {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
    }

    return () => {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
      // Do not reset startTimeRef — it's intentionally preserved
      // across re-renders to keep the elapsed duration stable.
    };
  }, [entry.argumentsComplete, entry.resultComplete]);

  // isExpanded is purely controlled by the `expanded` state
  const isExpanded = expanded;

  function handleToggle() {
    setExpanded((prev) => !prev);
  }

  // Check if output is JSON and should use tree view
  const output = entry.result?.output?.trim() || "";
  const parsedJson =
    !isBash(entry.name) &&
    !isWriteTool(entry.name) &&
    output &&
    looksLikeJson(output)
      ? tryParseJson(output)
      : null;

  const isRunning = !entry.resultComplete && entry.argumentsComplete;
  const showDuration = entry.resultComplete && elapsedMs > 0;

  return (
    <div
      className={`my-2 overflow-hidden rounded-lg border ${getToolBg(entry.name)}`}
    >
      {/* Collapsed Header (always visible) */}
      <button
        onClick={handleToggle}
        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-black/5 dark:hover:bg-white/5"
      >
        {(() => {
          const Icon = getToolIcon(entry.name);
          return (
            <Icon
              className={`h-3.5 w-3.5 flex-shrink-0 ${getToolColor(entry.name)}`}
            />
          );
        })()}

        {/* Collapsed label */}
        {isBash(entry.name) ? (
          <div className="flex flex-1 flex-col min-w-0">
            <span className="text-xs font-medium text-neutral-700 dark:text-neutral-300">
              bash
            </span>
            <span className="truncate font-mono text-xs text-neutral-500 dark:text-neutral-400">
              $ {getBashCommand(entry) || "..."}
            </span>
          </div>
        ) : (
          <span className="flex-1 truncate text-xs font-medium text-neutral-700 dark:text-neutral-300">
            {getCollapsedLabel(entry)}
          </span>
        )}

        {/* Status: spinner for in-progress, duration when done */}
        <div className="flex items-center gap-2 flex-shrink-0">
          {isRunning && (
            <>
              <Loader2 className="h-3.5 w-3.5 animate-spin text-neutral-400" />
              {elapsedMs > 0 && (
                <span className="text-xs tabular-nums text-neutral-400">
                  {formatDuration(elapsedMs)}
                </span>
              )}
            </>
          )}
          {showDuration && (
            <span className="flex items-center gap-1 text-xs text-neutral-400">
              <Clock className="h-3 w-3" />
              {formatDuration(elapsedMs)}
            </span>
          )}
          {entry.resultComplete && (
            <ChevronDown
              className={`h-3.5 w-3.5 text-neutral-400 transition-transform ${isExpanded ? "rotate-180" : ""}`}
            />
          )}
        </div>
      </button>

      {/* Expanded Content */}
      {isExpanded && entry.result && (
        <div className="border-t border-inherit">
          <div className="px-3 py-2">
            {/* Diff display for write/edit */}
            {isWriteTool(entry.name) && entry.result.diff && (
              <DiffRenderer
                diff={entry.result.diff}
                filepath={entry.result.filepath || ""}
              />
            )}

            {/* Bash: show description + command + output */}
            {isBash(entry.name) && (
              <div className="space-y-1">
                {getBashDescription(entry) && (
                  <div className="rounded bg-neutral-100 px-3 py-1.5 text-xs text-neutral-600 dark:bg-neutral-800 dark:text-neutral-400">
                    {getBashDescription(entry)}
                  </div>
                )}
                <div className="overflow-x-auto whitespace-pre-wrap break-all rounded bg-black/5 px-3 py-1.5 font-mono text-xs leading-relaxed text-neutral-600 dark:bg-white/5 dark:text-neutral-400">
                  $ {getBashCommand(entry)}
                </div>
                {entry.result.output && (
                  <pre className="overflow-x-auto whitespace-pre-wrap font-mono text-xs leading-relaxed text-neutral-700 dark:text-neutral-300">
                    {entry.result.output}
                  </pre>
                )}
              </div>
            )}

            {/* Read tool: render as code lines with line numbers */}
            {entry.name === "read" && (
              <CodeLinesRenderer
                output={entry.result.output}
                filepath={entry.result.filepath}
              />
            )}

            {/* Todo tool: render as structured list */}
            {isTodo(entry.name) && (
              <TodoRenderer output={entry.result.output} />
            )}

            {/* Other read-only tools: render as markdown or JSON tree */}
            {isReadOnlyTool(entry.name) &&
              entry.name !== "read" &&
              (parsedJson ? (
                <JsonTreeView data={parsedJson} initialExpanded={true} />
              ) : (
                <MarkdownRenderer content={entry.result.output} />
              ))}

            {/* websearch/webfetch: show JSON tree if applicable */}
            {isWebTool(entry.name) &&
              (parsedJson ? (
                <JsonTreeView
                  data={parsedJson}
                  initialExpanded={true}
                  maxDepth={5}
                />
              ) : (
                <MarkdownRenderer content={entry.result.output} />
              ))}

            {/* Default: render as markdown or JSON tree */}
            {!isReadOnlyTool(entry.name) &&
              !isBash(entry.name) &&
              !isWriteTool(entry.name) &&
              !isTodo(entry.name) &&
              !isWebTool(entry.name) &&
              (parsedJson ? (
                <JsonTreeView data={parsedJson} initialExpanded={true} />
              ) : (
                <MarkdownRenderer content={entry.result.output} />
              ))}
          </div>

          {/* Exit code for bash commands */}
          {isBash(entry.name) && getExitCode(entry) !== null && (
            <div className="border-t border-inherit bg-black/5 px-3 py-1 text-xs dark:bg-white/5">
              <span className="text-neutral-500 dark:text-neutral-400">
                Exit code: {getExitCode(entry)}
                {getExitCode(entry) === 0 ? (
                  <span className="text-green-600 dark:text-green-400">
                    {" "}
                    &#10003;
                  </span>
                ) : (
                  <span className="text-red-600 dark:text-red-400">
                    {" "}
                    &#10007;
                  </span>
                )}
              </span>
            </div>
          )}
        </div>
      )}
    </div>
  );
});
