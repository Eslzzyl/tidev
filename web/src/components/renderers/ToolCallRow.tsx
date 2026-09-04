import { memo, useEffect, useMemo, useRef, useState, type ComponentType } from "react";
import type { TFunction } from "i18next";
import {
  Clock,
  FileDiff,
  FileEdit,
  FileText,
  Files,
  LayoutTemplate,
  ListTodo,
  Search,
  Sparkles,
  Terminal,
  Wrench,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import type { MessageAttachment, ToolExecutionResult } from "../../types/api";
import type { ToolCallEntry } from "../../utils/round";
import { Button } from "../ui";
import { ExpandableBody } from "../ui/ExpandableBody";
import { JsonTreeView } from "../ui/JsonTreeView";
import { ActivityRipple } from "./ActivityRipple";
import { DiffRenderer } from "./DiffRenderer";
import { MarkdownRenderer } from "./MarkdownRenderer";
import { formatFileSize, ReadResultRenderer } from "./ReadResultRenderer";
import { SubagentCard } from "./SubagentCard";
import { TodoRenderer } from "./TodoRenderer";

interface Props {
  entry: ToolCallEntry;
  workspaceRoot?: string;
  expanded?: boolean;
  onExpandedChange?: (expanded: boolean) => void;
}

interface ToolCallBodyProps {
  entry: ToolCallEntry;
  args: ToolArguments;
  metadata: ToolExecutionResult["metadata"] | undefined;
  output: string;
  summary: string;
  t: TFunction;
}

const READ_ONLY_TOOLS = new Set(["read", "grep", "glob", "skill"]);
const WRITE_TOOLS = new Set(["write", "edit", "apply_patch"]);
const WEB_TOOLS = new Set(["websearch", "webfetch"]);
const TOOL_LABELS: Record<string, string> = {
  read: "Read file",
  grep: "Search text",
  glob: "Find files",
  write: "Write file",
  edit: "Edit file",
  apply_patch: "Edit file",
  skill: "Load skill",
  bash: "Run command",
  shell: "Run command",
  task: "Subagent",
  todowrite: "Update to-do",
  websearch: "Web search",
  webfetch: "Fetch web page",
  question: "Ask",
};

type ToolArguments = Record<string, unknown> | null;

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

export function parseMcpToolName(name: string): { server: string; tool: string } | null {
  if (!name.startsWith("mcp__")) return null;
  const parts = name.slice(5).split("__");
  if (parts.length >= 2 && parts[0] && parts[1]) {
    return { server: parts[0], tool: parts.slice(1).join("__") };
  }
  return null;
}

function toolLabel(entry: ToolCallEntry, t: TFunction, args: ToolArguments) {
  const mcp = parseMcpToolName(entry.name);
  if (mcp) {
    return `MCP ${mcp.server} ❯ ${mcp.tool}`;
  }
  if (entry.name === "grep" || entry.name === "glob") return "";
  if (entry.name === "skill") {
    const hasName = Boolean(stringArgument(args, "name"));
    return t(
      hasName && stringArgument(args, "path")
        ? "Read skill file"
        : hasName
          ? "Load skill"
          : "List skills",
    );
  }
  return t(TOOL_LABELS[entry.name] ?? "Tool call");
}

function parseArguments(entry: ToolCallEntry): ToolArguments {
  try {
    const value = JSON.parse(entry.arguments) as unknown;
    return value && typeof value === "object" ? (value as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

function stringArgument(args: ToolArguments, key: string) {
  const value = args?.[key];
  return typeof value === "string" ? value : "";
}

function summarizeArguments(
  entry: ToolCallEntry,
  workspaceRoot: string,
  t: TFunction,
  args: ToolArguments,
) {
  if (!args) return entry.arguments || "…";
  const mcp = parseMcpToolName(entry.name);
  if (mcp) {
    const entries = Object.entries(args);
    if (entries.length === 0) return "";
    if (entries.length === 1) {
      const [k, v] = entries[0];
      if (typeof v === "string") {
        return `${k}="${v.length > 60 ? v.slice(0, 57) + "..." : v}"`;
      }
      return `${k}=${JSON.stringify(v)}`;
    }
    const parts = entries.map(([k, v]) => {
      const str =
        typeof v === "string"
          ? `"${v.length > 35 ? v.slice(0, 32) + "..." : v}"`
          : JSON.stringify(v);
      return `${k}=${str}`;
    });
    const joined = parts.join(", ");
    return joined.length > 80 ? `${joined.slice(0, 77)}...` : joined;
  }
  switch (entry.name) {
    case "read":
    case "write":
    case "edit":
      return relativePath(
        stringArgument(args, "file_path") || stringArgument(args, "path") || t("Unknown"),
        workspaceRoot,
      );
    case "apply_patch":
      return (
        patchFileChanges(stringArgument(args, "patch_text"), workspaceRoot)
          .map((change) => change.path)
          .join(t("File change separator")) || t("Patch files")
      );
    case "grep": {
      const pattern = stringArgument(args, "pattern");
      const path = relativePath(stringArgument(args, "path") || ".", workspaceRoot);
      return pattern
        ? t('Search in {{path}} for "{{pattern}}"', { path, pattern })
        : t("Search in {{path}}", { path });
    }
    case "glob": {
      const pattern = stringArgument(args, "pattern") || "*";
      const path = relativePath(stringArgument(args, "path") || ".", workspaceRoot);
      return t("Find files in {{path}} matching {{pattern}}", { path, pattern });
    }
    case "bash":
    case "shell":
      return stringArgument(args, "command") || t("No command");
    case "skill":
      if (!stringArgument(args, "name")) return "";
      return stringArgument(args, "path")
        ? `${stringArgument(args, "name")}/${stringArgument(args, "path")}`
        : stringArgument(args, "name");
    case "task":
      return stringArgument(args, "description") || t("No description");
    case "todowrite": {
      const todos = args.todos;
      return Array.isArray(todos) ? t("{{count}} to-dos", { count: todos.length }) : t("No to-dos");
    }
    case "websearch":
      return stringArgument(args, "query") || t("No query");
    case "webfetch":
      return stringArgument(args, "url") || t("No URL");
    case "question": {
      const questions = args?.questions;
      if (!Array.isArray(questions)) return t("Unknown");
      const prompts = questions
        .map((item) => {
          if (typeof item === "string") return item;
          if (item && typeof item === "object") {
            const question = (item as Record<string, unknown>).question;
            return typeof question === "string" ? question : "";
          }
          return "";
        })
        .filter(Boolean);
      return prompts.join(t("File change separator")) || t("Unknown");
    }
    default:
      return entry.arguments.length > 80 ? `${entry.arguments.slice(0, 80)}...` : entry.arguments;
  }
}

function bashCommand(args: ToolArguments) {
  return stringArgument(args, "command");
}

function bashDescription(args: ToolArguments) {
  return stringArgument(args, "description");
}

function formatDuration(milliseconds: number) {
  if (milliseconds < 1000) return `${milliseconds}ms`;
  if (milliseconds < 60000) return `${(milliseconds / 1000).toFixed(1)}s`;
  return `${Math.floor(milliseconds / 60000)}m ${Math.floor((milliseconds % 60000) / 1000)}s`;
}

export function normalizeToolOutput(output: string): {
  data: unknown | null;
  text: string;
  isJson: boolean;
} {
  const trimmed = output.trim();
  if (!trimmed) return { data: null, text: "", isJson: false };
  try {
    const val = JSON.parse(trimmed) as unknown;
    if (val && typeof val === "object" && !Array.isArray(val)) {
      const record = val as Record<string, unknown>;
      const keys = Object.keys(record);
      if (keys.length === 1 && "result" in record) {
        const inner = record.result;
        if (typeof inner === "string") {
          const innerTrimmed = inner.trim();
          try {
            const innerVal = JSON.parse(innerTrimmed) as unknown;
            if (innerVal && typeof innerVal === "object") {
              return { data: innerVal, text: JSON.stringify(innerVal, null, 2), isJson: true };
            }
          } catch {
            return { data: null, text: inner, isJson: false };
          }
          return { data: null, text: inner, isJson: false };
        } else if (inner && typeof inner === "object") {
          return { data: inner, text: JSON.stringify(inner, null, 2), isJson: true };
        }
      }
      return { data: val, text: JSON.stringify(val, null, 2), isJson: true };
    } else if (Array.isArray(val)) {
      return { data: val, text: JSON.stringify(val, null, 2), isJson: true };
    }
  } catch {
    // not json
  }
  return { data: null, text: output, isJson: false };
}

const ToolCallBody = memo(function ToolCallBody({
  entry,
  args,
  metadata,
  output,
  summary,
  t,
}: ToolCallBodyProps) {
  const fileChanges = metadata?.file_changes.filter((change) => Boolean(change.diff)) ?? [];
  const hasDiff = Boolean(metadata?.diff) || fileChanges.length > 0;
  const normalized = useMemo(() => normalizeToolOutput(output), [output]);
  const parsedJson = !isWriteTool(entry.name) && !isBash(entry.name) ? normalized.data : null;
  const displayText = normalized.text;

  return (
    <div className="tool-renderer-body">
      {entry.result && isWriteTool(entry.name) && hasDiff ? (
        <div className="tool-file-diffs">
          {fileChanges.map((change) => (
            <DiffRenderer
              key={change.path}
              diff={change.diff ?? ""}
              filepath={change.path}
              compact
            />
          ))}
          {fileChanges.length === 0 && metadata?.diff ? (
            <DiffRenderer diff={metadata.diff} filepath={metadata.filepath || summary} compact />
          ) : null}
        </div>
      ) : null}
      {entry.result && isWriteTool(entry.name) && !hasDiff && displayText ? (
        <MarkdownRenderer content={displayText} />
      ) : null}
      {entry.result && isBash(entry.name) ? (
        <div className="tool-bash-output">
          <code className="tool-command">$ {bashCommand(args)}</code>
          {displayText ? <pre className="tool-raw-output">{displayText}</pre> : null}
        </div>
      ) : null}
      {entry.result && entry.name === "read" ? (
        <ReadResultRenderer
          output={entry.result.output}
          filepath={metadata?.filepath ?? undefined}
          attachments={entry.result.attachments}
        />
      ) : null}
      {entry.result && entry.name === "todowrite" ? <TodoRenderer output={displayText} /> : null}
      {entry.result && isReadOnlyTool(entry.name) && entry.name !== "read" ? (
        parsedJson ? (
          <JsonTreeView
            data={parsedJson}
            initialExpanded
            maxDepth={entry.name === "skill" ? 3 : 5}
            embedded
          />
        ) : (
          <MarkdownRenderer content={displayText} />
        )
      ) : null}
      {entry.result && WEB_TOOLS.has(entry.name) ? (
        parsedJson ? (
          <JsonTreeView data={parsedJson} initialExpanded maxDepth={5} embedded />
        ) : (
          <MarkdownRenderer content={displayText} />
        )
      ) : null}
      {entry.result &&
      !isReadOnlyTool(entry.name) &&
      !isWriteTool(entry.name) &&
      !isBash(entry.name) &&
      entry.name !== "todowrite" &&
      !WEB_TOOLS.has(entry.name) ? (
        parsedJson ? (
          <JsonTreeView data={parsedJson} initialExpanded embedded />
        ) : (
          <MarkdownRenderer content={displayText} />
        )
      ) : null}
      {entry.result && !displayText ? (
        <span className="tool-empty-output">{t("No output")}</span>
      ) : null}
      {entry.result &&
      isBash(entry.name) &&
      metadata?.exit_code !== null &&
      metadata?.exit_code !== undefined ? (
        <div className="tool-exit-code">
          {t("Exit code")}: {metadata.exit_code} {metadata.exit_code === 0 ? "✓" : "×"}
        </div>
      ) : null}
      {!entry.result ? (
        <pre className="tool-arguments">{entry.arguments || t("Waiting for arguments…")}</pre>
      ) : null}
    </div>
  );
});

function statusSuffix(entry: ToolCallEntry, t: TFunction) {
  if (entry.status === "failed") return t(", failed");
  if (entry.status === "pending") return t(", waiting");
  if (entry.status === "running") return t(", running");
  return t(", completed");
}

function parseRange(value: string): [number, number] | null {
  const match = value.trim().match(/^(\d+)\s*-\s*(\d+)$/);
  return match ? [Number(match[1]), Number(match[2])] : null;
}

function readMetadata(output: string): {
  lineRange: [number, number];
  requestedRange: [number, number] | null;
  total: number;
  truncatedBy: string | null;
} | null {
  let lineRange: [number, number] | null = null;
  let requestedRange: [number, number] | null = null;
  let total: number | null = null;
  let truncatedBy: string | null = null;

  for (const line of output.split("\n")) {
    const lineValue = line.match(/^<line_range>(.*?)<\/line_range>$/)?.[1];
    const requestedValue = line.match(/^<requested_range>(.*?)<\/requested_range>$/)?.[1];
    const totalValue = line.match(/^<file_total>(.*?)<\/file_total>$/)?.[1];
    const truncatedValue = line.match(/^<truncated_by>(.*?)<\/truncated_by>$/)?.[1];
    if (lineValue) lineRange = parseRange(lineValue);
    if (requestedValue) requestedRange = parseRange(requestedValue);
    if (totalValue && /^\d+$/.test(totalValue.trim())) total = Number(totalValue.trim());
    if (truncatedValue && (truncatedValue === "size" || truncatedValue === "lines")) {
      truncatedBy = truncatedValue;
    }
  }

  return lineRange && total !== null ? { lineRange, requestedRange, total, truncatedBy } : null;
}

function isErrorOutput(output: string) {
  return /^(Error:|User cancelled the request)/.test(output.trim());
}

function attachmentSummary(
  attachments: MessageAttachment[],
  output: string,
  t: TFunction,
): string | null {
  const image = attachments.find(
    (attachment): attachment is Extract<MessageAttachment, { type: "image" }> =>
      attachment.type === "image",
  );
  if (image) {
    const type = image.mime.replace(/^image\//, "");
    return t("{{type}}, {{size}}", { type, size: formatFileSize(image.file_size) });
  }

  if (attachments.some((attachment) => attachment.type === "directory_reference")) {
    let files = 0;
    let directories = 0;
    for (const line of output.split("\n").slice(1)) {
      const value = line.trim();
      if (!value) continue;
      if (value.endsWith("/")) directories += 1;
      else files += 1;
    }
    if (files === 0 && directories === 0) return t("Empty");
    if (directories === 0) return t("{{count}} files", { count: files });
    if (files === 0) return t("{{count}} directories", { count: directories });
    return t("{{files}} files, {{directories}} directories", { files, directories });
  }

  return null;
}

function readResultSummary(entry: ToolCallEntry, t: TFunction) {
  const output = entry.result?.output ?? "";
  const lower = output.toLowerCase();
  if (lower.includes("file not found")) {
    return lower.includes("did you mean")
      ? t("File not found with suggestions")
      : t("File not found");
  }
  if (lower.includes("escapes the workspace root") || lower.includes(" was denied")) {
    return t("Blocked by policy");
  }
  if (isErrorOutput(output)) return t("Tool error");

  const attachment = attachmentSummary(entry.result?.attachments ?? [], output, t);
  if (attachment) return attachment;

  const metadata = readMetadata(output);
  const sizeTruncated = output.includes("Output capped at 50 KB");
  const outputTruncated =
    sizeTruncated ||
    metadata?.truncatedBy === "size" ||
    metadata?.truncatedBy === "lines" ||
    entry.result?.metadata.truncated === true;
  const legacyRange = output.match(/Showing lines (\d+)-(\d+)(?: of (\d+))?/i);
  if (metadata) {
    const [start, end] = metadata.lineRange;
    const isFullFile = start === 1 && end === metadata.total;
    const requestedRange =
      metadata.requestedRange &&
      end - start + 1 !== metadata.requestedRange[1] - metadata.requestedRange[0] + 1
        ? metadata.requestedRange
        : null;
    let summary = isFullFile
      ? t("Read all {{count}} lines", { count: metadata.total })
      : requestedRange
        ? t(
            "Read lines {{start}}-{{end}} of {{total}} (requested {{requestedStart}}-{{requestedEnd}})",
            {
              start,
              end,
              total: metadata.total,
              requestedStart: requestedRange[0],
              requestedEnd: requestedRange[1],
            },
          )
        : t("Read lines {{start}}-{{end}} of {{total}}", {
            start,
            end,
            total: metadata.total,
          });
    const requestSatisfied = metadata.requestedRange && end >= metadata.requestedRange[1];
    if (!isFullFile && outputTruncated && !requestSatisfied) {
      summary = readDetailSummary(summary, t("truncated by 50 KB"), t);
    } else if (metadata.truncatedBy === "lines") {
      summary = readDetailSummary(summary, t("truncated by 2000 lines"), t);
    }
    return summary;
  }

  if (legacyRange) {
    const [, start, end, total] = legacyRange;
    const summary = total
      ? t("Read lines {{start}}-{{end}} of {{total}}", {
          start,
          end,
          total,
        })
      : t("Read lines {{start}}-{{end}}", { start, end });
    return outputTruncated ? readDetailSummary(summary, t("truncated"), t) : summary;
  }

  const lines = output ? output.split("\n").length : 0;
  if (lines === 0) return t("Empty");
  return outputTruncated
    ? t("{{count}} lines (truncated)", { count: lines })
    : t("{{count}} lines", { count: lines });
}

function readDetailSummary(summary: string, detail: string, t: TFunction) {
  return t("Result detail", { summary, detail });
}

function skillResultSummary(entry: ToolCallEntry, t: TFunction) {
  const output = entry.result?.output ?? "";
  const lower = output.toLowerCase();
  if (lower.includes("unknown skill '")) return t("Skill not found");
  if (lower.includes("file not found")) return t("File not found");
  if (
    lower.includes("escapes the skill directory") ||
    lower.includes("path must be relative to the skill directory")
  ) {
    return t("Blocked by policy");
  }
  if (lower.includes("a path requires a skill name") || lower.includes("path must not be empty")) {
    return t("Invalid request");
  }
  if (lower.includes("cannot read binary file")) return t("Binary file");
  if (output.startsWith("Available skills")) {
    const total = output.match(/of\s+(\d+)/i)?.[1];
    return total ? t("{{count}} skills", { count: total }) : t("Skills listed");
  }
  const lines = output
    .split("\n")
    .filter((line) => line.trim() && !line.startsWith("#") && !line.startsWith("**"));
  return t("{{count}} lines", { count: lines.length });
}

function diffLineCounts(
  diff: string | null | undefined,
): { additions: number; deletions: number } | null {
  if (!diff) return null;
  const lines = diff.split("\n");
  const looksLikeDiff = lines.some(
    (line) =>
      line.startsWith("diff --git ") ||
      line.startsWith("--- ") ||
      line.startsWith("+++ ") ||
      line.startsWith("@@"),
  );
  if (!looksLikeDiff) return null;
  let additions = 0;
  let deletions = 0;
  for (const line of lines) {
    if (line.startsWith("+") && !line.startsWith("+++")) additions += 1;
    if (line.startsWith("-") && !line.startsWith("---")) deletions += 1;
  }
  return { additions, deletions };
}

function textLineCount(value: string) {
  const normalized = value.replace(/\n$/, "");
  return normalized ? normalized.split("\n").length : 0;
}

function diffCountLabel(counts: { additions: number; deletions: number } | null) {
  if (!counts || (counts.additions === 0 && counts.deletions === 0)) return "";
  const parts: string[] = [];
  if (counts.additions > 0) parts.push(`+${counts.additions}`);
  if (counts.deletions > 0) parts.push(`-${counts.deletions}`);
  return parts.join(" ");
}

interface InlineFileChange {
  path: string;
  counts: { additions: number; deletions: number } | null;
}

function patchFileChanges(patchText: string, workspaceRoot: string): InlineFileChange[] {
  const changes: InlineFileChange[] = [];
  let current: InlineFileChange | null = null;
  for (const line of patchText.split("\n")) {
    const header = line.match(/^\*\*\* (Add|Update|Delete) File:\s*(.+)$/);
    if (header) {
      current = {
        path: relativePath(header[2].trim(), workspaceRoot),
        counts: { additions: 0, deletions: 0 },
      };
      changes.push(current);
      continue;
    }
    if (!current) continue;
    if (line.startsWith("+") && !line.startsWith("+++")) current.counts!.additions += 1;
    if (line.startsWith("-") && !line.startsWith("---")) current.counts!.deletions += 1;
  }
  return changes.map((change) => ({
    ...change,
    counts:
      change.counts && change.counts.additions + change.counts.deletions > 0 ? change.counts : null,
  }));
}

function inlineFileChanges(
  entry: ToolCallEntry,
  workspaceRoot: string,
  unknownPath: string,
  args: ToolArguments,
): InlineFileChange[] {
  const metadata = entry.result?.metadata;
  if (metadata?.file_changes.length) {
    return metadata.file_changes.map((change) => ({
      path: relativePath(change.path, workspaceRoot),
      counts: diffLineCounts(change.diff),
    }));
  }
  if (metadata?.diff) {
    return [
      {
        path: relativePath(
          metadata.filepath || stringArgument(args, "file_path") || unknownPath,
          workspaceRoot,
        ),
        counts: diffLineCounts(metadata.diff),
      },
    ];
  }
  if (entry.result) return [];
  if (entry.name === "apply_patch") {
    return patchFileChanges(stringArgument(args, "patch_text"), workspaceRoot);
  }
  if (entry.name === "write") {
    const path = stringArgument(args, "file_path") || stringArgument(args, "path");
    return path
      ? [
          {
            path: relativePath(path, workspaceRoot),
            counts: { additions: textLineCount(stringArgument(args, "content")), deletions: 0 },
          },
        ]
      : [];
  }
  if (entry.name === "edit") {
    const path = stringArgument(args, "file_path") || stringArgument(args, "path");
    return path
      ? [
          {
            path: relativePath(path, workspaceRoot),
            counts: {
              additions: textLineCount(
                stringArgument(args, "new_text") || stringArgument(args, "new_string"),
              ),
              deletions: textLineCount(
                stringArgument(args, "old_text") || stringArgument(args, "old_string"),
              ),
            },
          },
        ]
      : [];
  }
  return [];
}

function fileChangeSummary(
  entry: ToolCallEntry,
  workspaceRoot: string,
  t: TFunction,
  args: ToolArguments,
  includePaths = true,
) {
  return inlineFileChanges(entry, workspaceRoot, t("Unknown"), args)
    .map((change) => {
      const counts = diffCountLabel(change.counts);
      if (counts && !includePaths) return counts;
      return counts ? `${change.path} ${counts}` : change.path;
    })
    .join(t("File change separator"));
}

function resultSummary(
  entry: ToolCallEntry,
  workspaceRoot: string,
  t: TFunction,
  args: ToolArguments,
) {
  const resultSuffix = (summary: string) => t("Result suffix", { summary });
  if (entry.name === "grep" || entry.name === "glob") {
    if (entry.status !== "completed" || !entry.result) return statusSuffix(entry, t);
    const firstLine = entry.result.output.split("\n")[0]?.trim() || "";
    if (/no files found/i.test(firstLine)) return resultSuffix(t("No results"));
    const count = firstLine.match(/Found (\d+)/i)?.[1];
    return count ? resultSuffix(t("Found {{count}} items", { count })) : statusSuffix(entry, t);
  }
  if (entry.name === "read" && entry.result) return resultSuffix(readResultSummary(entry, t));
  if (entry.name === "skill" && entry.result) return resultSuffix(skillResultSummary(entry, t));
  if (entry.status !== "completed" || !entry.result) return statusSuffix(entry, t);
  if (isWriteTool(entry.name)) {
    const changes = fileChangeSummary(entry, workspaceRoot, t, args, entry.name === "apply_patch");
    return changes ? resultSuffix(changes) : statusSuffix(entry, t);
  }
  return statusSuffix(entry, t);
}

export const ToolCallRow = memo(function ToolCallRow({
  entry,
  workspaceRoot = "",
  expanded: controlledExpanded,
  onExpandedChange,
}: Props) {
  const { t } = useTranslation();
  const [localExpanded, setLocalExpanded] = useState(false);
  const [elapsedMs, setElapsedMs] = useState(0);
  const startTime = useRef<number | null>(null);
  const expanded = controlledExpanded ?? localExpanded;

  const output = entry.result?.output?.trim() || "";
  const active = entry.status === "pending" || entry.status === "running";
  const running = entry.status === "running";
  const metadata = entry.result?.metadata;
  const Icon = toolIcon(entry.name);
  const args = useMemo(() => parseArguments(entry), [entry.arguments]);
  const tone = toolTone(entry.name);
  const summary = useMemo(
    () => summarizeArguments(entry, workspaceRoot, t, args),
    [args, entry, t, workspaceRoot],
  );
  const label = useMemo(() => toolLabel(entry, t, args), [args, entry, t]);
  const result = useMemo(
    () => resultSummary(entry, workspaceRoot, t, args),
    [args, entry, t, workspaceRoot],
  );

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
    return (
      <SubagentCard
        entry={entry}
        expanded={expanded}
        onExpandedChange={(next) => {
          if (controlledExpanded === undefined) setLocalExpanded(next);
          onExpandedChange?.(next);
        }}
      />
    );
  }

  const measuredDurationMs =
    metadata?.duration_ms && metadata.duration_ms > 0
      ? metadata.duration_ms
      : elapsedMs > 0
        ? elapsedMs
        : null;
  const duration = measuredDurationMs === null ? "" : formatDuration(measuredDurationMs);
  const description = isBash(entry.name) ? bashDescription(args) : "";
  const inlineSummary = isBash(entry.name)
    ? description || `$ ${bashCommand(args) || t("Unknown")}`
    : summary;
  const inlineSummaryIsDescription = isBash(entry.name) && Boolean(description);

  return (
    <div className={`tool-renderer tool-tone-${tone}`}>
      <Button
        type="button"
        className="tool-renderer-header"
        onClick={() => {
          const next = !expanded;
          if (controlledExpanded === undefined) setLocalExpanded(next);
          onExpandedChange?.(next);
        }}
        aria-expanded={expanded}
        variant="ghost"
        size="sm"
      >
        <ActivityRipple
          active={active}
          row
          label={t(entry.status === "pending" ? "Tool is waiting" : "Tool is running")}
        >
          <Icon size={14} />
          <span className="tool-renderer-title">
            {label ? <strong>{label}</strong> : null}
            {inlineSummaryIsDescription ? (
              <span>
                {inlineSummary}
                {result}
              </span>
            ) : (
              <code>
                {inlineSummary}
                {result}
              </code>
            )}
          </span>
          <span className="tool-renderer-status">
            {entry.status === "failed" ? <X size={14} aria-label={t("Failed")} /> : null}
            {duration && (isBash(entry.name) || !running) ? (
              <>
                <Clock size={12} />
                {duration}
              </>
            ) : null}
          </span>
        </ActivityRipple>
      </Button>
      <ExpandableBody expanded={expanded} className="tool-renderer-body-shell">
        <ToolCallBody
          entry={entry}
          args={args}
          metadata={metadata}
          output={output}
          summary={summary}
          t={t}
        />
      </ExpandableBody>
    </div>
  );
});
