import { useState } from "react";
import { ChevronRight, ChevronDown } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "./Button";

interface JsonTreeViewProps {
  data: unknown;
  initialExpanded?: boolean;
  maxDepth?: number;
  embedded?: boolean;
}

type JsonType = "string" | "number" | "boolean" | "null" | "array" | "object";

function getType(value: unknown): JsonType {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  return typeof value as JsonType;
}

function getValueColor(type: JsonType, embedded: boolean): string {
  switch (type) {
    case "string":
      return "text-emerald-600 dark:text-emerald-400";
    case "number":
      return embedded
        ? "text-neutral-600 dark:text-neutral-400"
        : "text-blue-600 dark:text-blue-400";
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
  embedded: boolean;
}

function TreeNode({ keyName, value, depth, maxDepth, embedded }: TreeNodeProps) {
  const { t } = useTranslation();
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
      <div className="tool-json-node">
        <Button
          type="button"
          onClick={() => setIsExpanded(!isExpanded)}
          className="tool-json-node-toggle"
          variant="ghost"
          size="sm"
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
              : `${bracket[0]} ${entries.length} ${type === "object" ? t("keys") : t("items")} ${bracket[1]}`}
          </span>
        </Button>
        {isExpanded && !empty && (
          <div
            className={
              embedded ? "ml-4" : "ml-4 border-l border-neutral-200 pl-3 dark:border-neutral-700"
            }
          >
            {entries.map(([k, v], _i) => (
              <TreeNode
                key={k}
                keyName={type === "object" ? k : null}
                value={v}
                depth={depth + 1}
                maxDepth={maxDepth}
                embedded={embedded}
              />
            ))}
          </div>
        )}
      </div>
    );
  }

  // Leaf node
  return (
    <div className="tool-json-node">
      <span className="inline-flex items-center gap-0.5">
        <span className="w-3.5" />
        {keyName !== null && <span className="text-neutral-500">&ldquo;{keyName}&rdquo;: </span>}
        <span className={getValueColor(type, embedded)}>{formatValue(value, type)}</span>
      </span>
    </div>
  );
}

export function JsonTreeView({
  data,
  initialExpanded = false,
  maxDepth = 3,
  embedded = false,
}: JsonTreeViewProps) {
  const { t } = useTranslation();
  const [isExpanded, setIsExpanded] = useState(initialExpanded);

  if (!data || typeof data !== "object") {
    const type = getType(data);
    return (
      <div className="tool-json-node">
        <span className={getValueColor(type, embedded)}>{formatValue(data, type)}</span>
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
    <div
      className={embedded ? "tool-json-tree" : "rounded bg-neutral-50 p-2 dark:bg-neutral-900/50"}
    >
      <Button
        type="button"
        onClick={() => setIsExpanded(!isExpanded)}
        className="tool-json-toggle inline-flex items-center gap-1 rounded px-1 py-0.5 text-neutral-600 hover:bg-neutral-200/50 dark:text-neutral-400 dark:hover:bg-neutral-800"
        variant="ghost"
        size="sm"
      >
        {isExpanded ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
        {type === "object" ? t("JSON") : t("Array")} ({entries.length}{" "}
        {type === "object" ? t("keys") : t("items")})
      </Button>
      {isExpanded && (
        <div className="tool-json-content mt-1">
          <span className="text-neutral-500">{bracket[0]}</span>
          <div
            className={
              embedded ? "ml-3" : "ml-3 border-l border-neutral-200 pl-2 dark:border-neutral-700"
            }
          >
            {entries.map(([k, v], _i) => (
              <TreeNode
                key={k}
                keyName={type === "object" ? k : null}
                value={v}
                depth={0}
                maxDepth={maxDepth}
                embedded={embedded}
              />
            ))}
          </div>
          <span className="text-neutral-500">{bracket[1]}</span>
        </div>
      )}
    </div>
  );
}
