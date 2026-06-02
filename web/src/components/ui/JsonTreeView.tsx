import { useState } from "react";
import { ChevronRight, ChevronDown } from "lucide-react";

interface JsonTreeViewProps {
  data: unknown;
  initialExpanded?: boolean;
  maxDepth?: number;
}

type JsonType = "string" | "number" | "boolean" | "null" | "array" | "object";

function getType(value: unknown): JsonType {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  return typeof value as JsonType;
}

function getValueColor(type: JsonType): string {
  switch (type) {
    case "string":
      return "text-emerald-600 dark:text-emerald-400";
    case "number":
      return "text-blue-600 dark:text-blue-400";
    case "boolean":
      return "text-violet-600 dark:text-violet-400";
    case "null":
      return "text-neutral-400 dark:text-neutral-500";
    default:
      return "text-neutral-900 dark:text-neutral-100";
  }
}

function formatValue(value: unknown, type: JsonType): string {
  switch (type) {
    case "string":
      return `"${String(value)}"`;
    case "null":
      return "null";
    default:
      return String(value);
  }
}

interface TreeNodeProps {
  keyName: string | null;
  value: unknown;
  depth: number;
  maxDepth: number;
}

function TreeNode({ keyName, value, depth, maxDepth }: TreeNodeProps) {
  const [isExpanded, setIsExpanded] = useState(depth < maxDepth);
  const type = getType(value);
  const isCollapsible = (type === "array" || type === "object") && depth < 10;

  if (isCollapsible) {
    const entries =
      type === "object"
        ? Object.entries(value as Record<string, unknown>)
        : (value as unknown[]).map((v, i) => [String(i), v] as const);
    const bracket = type === "object" ? ["{", "}"] : ["[", "]"];
    const empty = entries.length === 0;

    return (
      <div className="font-mono text-xs leading-5">
        <button
          onClick={() => setIsExpanded(!isExpanded)}
          className="inline-flex items-center gap-0.5 rounded px-0.5 hover:bg-neutral-100 dark:hover:bg-neutral-800"
        >
          {empty ? (
            <span className="w-3.5" />
          ) : isExpanded ? (
            <ChevronDown className="h-3 w-3 text-neutral-400" />
          ) : (
            <ChevronRight className="h-3 w-3 text-neutral-400" />
          )}
          {keyName !== null && (
            <>
              <span className="text-neutral-500">&ldquo;{keyName}&rdquo;: </span>
            </>
          )}
          <span className="text-neutral-500">
            {isExpanded
              ? ""
              : `${bracket[0]} ${entries.length} ${type === "object" ? "keys" : "items"} ${bracket[1]}`}
          </span>
        </button>
        {isExpanded && !empty && (
          <div className="ml-4 border-l border-neutral-200 pl-3 dark:border-neutral-700">
            {entries.map(([k, v], i) => (
              <TreeNode
                key={k}
                keyName={type === "object" ? k : null}
                value={v}
                depth={depth + 1}
                maxDepth={maxDepth}
                isLast={i === entries.length - 1}
              />
            ))}
          </div>
        )}
      </div>
    );
  }

  // Leaf node
  return (
    <div className="font-mono text-xs leading-5">
      <span className="inline-flex items-center gap-0.5">
        <span className="w-3.5" />
        {keyName !== null && <span className="text-neutral-500">&ldquo;{keyName}&rdquo;: </span>}
        <span className={getValueColor(type)}>{formatValue(value, type)}</span>
      </span>
    </div>
  );
}

export function JsonTreeView({ data, initialExpanded = false, maxDepth = 3 }: JsonTreeViewProps) {
  const [isExpanded, setIsExpanded] = useState(initialExpanded);

  if (!data || typeof data !== "object") {
    const type = getType(data);
    return (
      <div className="font-mono text-xs leading-5">
        <span className={getValueColor(type)}>{formatValue(data, type)}</span>
      </div>
    );
  }

  const type = getType(data);
  const entries =
    type === "object"
      ? Object.entries(data as Record<string, unknown>)
      : (data as unknown[]).map((v, i) => [String(i), v] as const);
  const bracket = type === "object" ? ["{", "}"] : ["[", "]"];

  return (
    <div className="rounded bg-neutral-50 p-2 dark:bg-neutral-900/50">
      <button
        onClick={() => setIsExpanded(!isExpanded)}
        className="inline-flex items-center gap-1 rounded px-1 py-0.5 text-xs font-medium text-neutral-600 hover:bg-neutral-200/50 dark:text-neutral-400 dark:hover:bg-neutral-800"
      >
        {isExpanded ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
        {type === "object" ? "JSON" : "Array"} ({entries.length}{" "}
        {type === "object" ? "keys" : "items"})
      </button>
      {isExpanded && (
        <div className="mt-1 font-mono text-xs leading-5">
          <span className="text-neutral-500">{bracket[0]}</span>
          <div className="ml-3 border-l border-neutral-200 pl-2 dark:border-neutral-700">
            {entries.map(([k, v], i) => (
              <TreeNode
                key={k}
                keyName={type === "object" ? k : null}
                value={v}
                depth={0}
                maxDepth={maxDepth}
                isLast={i === entries.length - 1}
              />
            ))}
          </div>
          <span className="text-neutral-500">{bracket[1]}</span>
        </div>
      )}
    </div>
  );
}
