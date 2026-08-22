import { memo, useEffect, useRef, useState, type ComponentType } from "react";
import {
  ChevronDown,
  Clock,
  FileDiff,
  FileEdit,
  FileText,
  Files,
  LayoutTemplate,
  ListTodo,
  Loader2,
  Search,
  Sparkles,
  Terminal,
  Wrench,
} from "lucide-react";

import type { ToolCallEntry } from "../../utils/round";
import { CodeLinesRenderer } from "./CodeLinesRenderer";
import { DiffRenderer } from "./DiffRenderer";
import { MarkdownRenderer } from "./MarkdownRenderer";
import { SubagentCard } from "./SubagentCard";
import { TodoRenderer } from "./TodoRenderer";
import { JsonTreeView } from "../ui/JsonTreeView";

interface Props {
  entry: ToolCallEntry;
  workspaceRoot?: string;
  defaultExpanded?: boolean;
}

const READ_ONLY_TOOLS = new Set(["read", "grep", "glob", "skill"]);
const WRITE_TOOLS = new Set(["write", "edit", "apply_patch"]);
const WEB_TOOLS = new Set(["websearch", "webfetch"]);

function isBash(name: string) {
  return name === "bash" || name === "shell";
}

function isReadOnlyTool(name: string) {
  return READ_ONLY_TOOLS.has(name);
}

function isWriteTool(name: string) {
  return WRITE_TOOLS.has(name);
}

function stripWindowsPrefix(value: string) {
  return value.replace(/^\\\\\?\\/, "");
}

function relativePath(value: string, workspaceRoot: string) {
  if (!value || !workspaceRoot) return value;
  const path = stripWindowsPrefix(value).replace(/\\/g, "/");
  const root = stripWindowsPrefix(workspaceRoot).replace(/\\/g, "/").replace(/\/$/, "");
  if (path === root) return ".";
  return path.startsWith(`${root}/`) ? path.slice(root.length + 1) : value;
}

function toolIcon(name: string): ComponentType<{ size?: number; className?: string }> {
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
    case "shell":
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

function toolTone(name: string) {
  if (isReadOnlyTool(name)) return "read";
  if (isWriteTool(name)) return "write";
  if (isBash(name)) return "bash";
  if (name === "task") return "task";
  if (name === "todowrite") return "todo";
  if (WEB_TOOLS.has(name)) return "web";
  return "default";
}

function parseArguments(entry: ToolCallEntry): Record<string, unknown> | null {
  try {
    const value = JSON.parse(entry.arguments) as unknown;
    return value && typeof value === "object" ? (value as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

function stringArgument(args: Record<string, unknown> | null, key: string) {
  const value = args?.[key];
  return typeof value === "string" ? value : "";
}

function summarizeArguments(entry: ToolCallEntry, workspaceRoot: string) {
  const args = parseArguments(entry);
  if (!args) return entry.arguments || "...";
  switch (entry.name) {
    case "read":
    case "write":
    case "edit":
      return relativePath(
        stringArgument(args, "file_path") || stringArgument(args, "path") || "(unknown)",
        workspaceRoot,
      );
    case "apply_patch":
      return relativePath(stringArgument(args, "path") || "(unknown)", workspaceRoot);
    case "grep": {
      const pattern = stringArgument(args, "pattern");
      const path = relativePath(stringArgument(args, "path") || ".", workspaceRoot);
      return pattern ? `"${pattern}" in ${path}` : path;
    }
    case "glob":
      return `${stringArgument(args, "pattern") || "*"} in ${relativePath(stringArgument(args, "path") || ".", workspaceRoot)}`;
    case "bash":
    case "shell":
      return stringArgument(args, "command") || "(no command)";
    case "skill":
      return stringArgument(args, "name") || "(unknown)";
    case "task":
      return stringArgument(args, "description") || "(no description)";
    case "todowrite": {
      const todos = args.todos;
      return Array.isArray(todos) ? `${todos.length} item(s)` : "(todos)";
    }
    case "websearch":
      return stringArgument(args, "query") || "(no query)";
    case "webfetch":
      return stringArgument(args, "url") || "(no url)";
    default:
      return entry.arguments.length > 80 ? `${entry.arguments.slice(0, 80)}...` : entry.arguments;
  }
}

function bashCommand(entry: ToolCallEntry) {
  return stringArgument(parseArguments(entry), "command");
}

function bashDescription(entry: ToolCallEntry) {
  return stringArgument(parseArguments(entry), "description");
}

function formatDuration(milliseconds: number) {
  if (milliseconds < 1000) return `${milliseconds}ms`;
  if (milliseconds < 60000) return `${(milliseconds / 1000).toFixed(1)}s`;
  return `${Math.floor(milliseconds / 60000)}m ${Math.floor((milliseconds % 60000) / 1000)}s`;
}

function parseJson(output: string): unknown | null {
  const trimmed = output.trim();
  if (!trimmed.startsWith("[") && !trimmed.startsWith("{")) return null;
  try {
    const value = JSON.parse(output) as unknown;
    return value && typeof value === "object" ? value : null;
  } catch {
    return null;
  }
}

function resultSummary(entry: ToolCallEntry) {
  if (!entry.result) return "";
  if (entry.result.isError) return " · failed";
  const firstLine = entry.result.output.split("\n")[0]?.trim() || "empty";
  if (entry.name === "grep" || entry.name === "glob") {
    if (/no files found/i.test(firstLine)) return " · no match";
    const count = firstLine.match(/Found (\d+)/i)?.[1];
    return count ? ` · ${count} match${count === "1" ? "" : "es"}` : ` · ${firstLine}`;
  }
  return ` · ${firstLine.length > 70 ? `${firstLine.slice(0, 70)}...` : firstLine}`;
}

export const ToolCallRow = memo(function ToolCallRow({
  entry,
  workspaceRoot = "",
  defaultExpanded = false,
}: Props) {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const [elapsedMs, setElapsedMs] = useState(0);
  const autoExpanded = useRef(false);
  const startTime = useRef<number | null>(null);

  const output = entry.result?.output?.trim() || "";
  const running = !entry.resultComplete && entry.argumentsComplete;
  const hasUsefulOutput = Boolean(output && output !== "Done");
  const hasDiff = Boolean(entry.result?.diff);
  const Icon = toolIcon(entry.name);

  useEffect(() => {
    if (autoExpanded.current) return;
    if (
      (isBash(entry.name) && entry.resultComplete && hasUsefulOutput) ||
      (hasDiff && entry.resultComplete)
    ) {
      autoExpanded.current = true;
      setExpanded(true);
    }
  }, [entry.name, entry.resultComplete, hasDiff, hasUsefulOutput]);

  useEffect(() => {
    if (!running) return;
    startTime.current ??= Date.now();
    const timer = setInterval(
      () => setElapsedMs(Date.now() - (startTime.current ?? Date.now())),
      100,
    );
    return () => clearInterval(timer);
  }, [running]);

  if (entry.name === "task") {
    return <SubagentCard entry={entry} />;
  }

  const parsedJson = !isWriteTool(entry.name) && !isBash(entry.name) ? parseJson(output) : null;
  const tone = toolTone(entry.name);
  const summary = summarizeArguments(entry, workspaceRoot);
  const duration = elapsedMs > 0 ? formatDuration(elapsedMs) : "";

  return (
    <div className={`tool-renderer tool-tone-${tone}`}>
      <button className="tool-renderer-header" onClick={() => setExpanded((value) => !value)}>
        <Icon size={14} />
        <span className="tool-renderer-title">
          {isBash(entry.name) ? <strong>bash</strong> : <strong>{entry.name}</strong>}
          <code>{isBash(entry.name) ? `$ ${bashCommand(entry) || "..."}` : summary}</code>
        </span>
        <span className="tool-renderer-status">
          {running ? <Loader2 className="spin" size={14} /> : null}
          {duration && !running ? (
            <>
              <Clock size={12} />
              {duration}
            </>
          ) : null}
          {entry.resultComplete ? (
            <ChevronDown
              className={expanded ? "thinking-chevron expanded" : "thinking-chevron"}
              size={14}
            />
          ) : null}
        </span>
      </button>
      {expanded && entry.result ? (
        <div className="tool-renderer-body">
          {isWriteTool(entry.name) && entry.result.diff ? (
            <DiffRenderer
              diff={entry.result.diff}
              filepath={entry.result.filepath || summary}
              compact
            />
          ) : null}
          {isBash(entry.name) ? (
            <div className="tool-bash-output">
              {bashDescription(entry) ? (
                <p className="tool-description">{bashDescription(entry)}</p>
              ) : null}
              <code className="tool-command">$ {bashCommand(entry)}</code>
              {entry.result.output ? (
                <pre className="tool-raw-output">{entry.result.output}</pre>
              ) : null}
            </div>
          ) : null}
          {entry.name === "read" ? (
            <CodeLinesRenderer output={entry.result.output} filepath={entry.result.filepath} />
          ) : null}
          {entry.name === "todowrite" ? <TodoRenderer output={entry.result.output} /> : null}
          {isReadOnlyTool(entry.name) && entry.name !== "read" ? (
            parsedJson ? (
              <JsonTreeView
                data={parsedJson}
                initialExpanded
                maxDepth={entry.name === "skill" ? 3 : 5}
              />
            ) : (
              <MarkdownRenderer content={entry.result.output} />
            )
          ) : null}
          {WEB_TOOLS.has(entry.name) ? (
            parsedJson ? (
              <JsonTreeView data={parsedJson} initialExpanded maxDepth={5} />
            ) : (
              <MarkdownRenderer content={entry.result.output} />
            )
          ) : null}
          {!isReadOnlyTool(entry.name) &&
          !isWriteTool(entry.name) &&
          !isBash(entry.name) &&
          entry.name !== "todowrite" &&
          !WEB_TOOLS.has(entry.name) ? (
            parsedJson ? (
              <JsonTreeView data={parsedJson} initialExpanded />
            ) : (
              <MarkdownRenderer content={entry.result.output} />
            )
          ) : null}
          {!entry.result.output ? <span className="tool-empty-output">No output</span> : null}
          {isBash(entry.name) &&
          entry.result.exitCode !== null &&
          entry.result.exitCode !== undefined ? (
            <div className="tool-exit-code">
              Exit code: {entry.result.exitCode} {entry.result.exitCode === 0 ? "✓" : "×"}
            </div>
          ) : null}
        </div>
      ) : null}
      {!entry.resultComplete && entry.result ? (
        <div className="tool-streaming-line">Updating result…</div>
      ) : null}
      {entry.resultComplete && !expanded ? (
        <span className="tool-collapsed-summary">{resultSummary(entry)}</span>
      ) : null}
    </div>
  );
});
