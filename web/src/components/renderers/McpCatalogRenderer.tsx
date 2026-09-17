import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { JsonTreeView } from "../ui/JsonTreeView";

type ToolArguments = Record<string, unknown> | null;

export interface McpServerRecord {
  server: string;
  kind: string;
  status: string;
  tool_count: number;
  error?: string;
}

export interface McpToolRecord {
  server: string;
  tool: string;
  description: string;
  input_schema: unknown;
  read_only: boolean;
}

export type McpCatalog =
  | { kind: "servers"; records: McpServerRecord[] }
  | { kind: "tools"; records: McpToolRecord[] };

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function isServerRecord(value: unknown): value is McpServerRecord {
  return (
    isRecord(value) &&
    typeof value.server === "string" &&
    typeof value.kind === "string" &&
    typeof value.status === "string" &&
    typeof value.tool_count === "number" &&
    (value.error === undefined || typeof value.error === "string")
  );
}

function isToolRecord(value: unknown): value is McpToolRecord {
  return (
    isRecord(value) &&
    typeof value.server === "string" &&
    typeof value.tool === "string" &&
    typeof value.description === "string" &&
    "input_schema" in value &&
    typeof value.read_only === "boolean"
  );
}

function parseJsonLines(output: string): unknown[] | null {
  const lines = output
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
  try {
    return lines.map((line) => JSON.parse(line) as unknown);
  } catch {
    return null;
  }
}

function serverArgument(args: ToolArguments) {
  const server = args?.server;
  return typeof server === "string" && server.trim() ? server.trim() : null;
}

export function parseMcpCatalogOutput(
  toolName: string,
  args: ToolArguments,
  output: string,
): McpCatalog | null {
  if (toolName !== "mcp_list" && toolName !== "mcp_search") return null;
  const records = parseJsonLines(output);
  if (!records) return null;

  const expectsServers = toolName === "mcp_list" && !serverArgument(args);
  if (expectsServers) {
    return records.every(isServerRecord) ? { kind: "servers", records } : null;
  }
  return records.every(isToolRecord) ? { kind: "tools", records } : null;
}

function statusClass(status: string) {
  return `mcp-catalog-status mcp-catalog-status-${status.toLowerCase().replace(/[^a-z0-9]+/g, "-")}`;
}

function ServerCatalog({ records }: { records: McpServerRecord[] }) {
  const { t } = useTranslation();
  if (records.length === 0) {
    return <span className="tool-empty-output">{t("No MCP servers")}</span>;
  }
  return (
    <div className="mcp-catalog-list">
      {records.map((record) => (
        <div className="mcp-catalog-server" key={record.server}>
          <div className="mcp-catalog-server-title">
            <span className={statusClass(record.status)}>{record.status}</span>
            <code>{record.server}</code>
            <span className="mcp-catalog-meta">
              {record.kind} · {t("{{count}} tools", { count: record.tool_count })}
            </span>
          </div>
          {record.error ? <p className="mcp-catalog-error">{record.error}</p> : null}
        </div>
      ))}
    </div>
  );
}

function ToolCatalog({ records, emptyLabel }: { records: McpToolRecord[]; emptyLabel: string }) {
  const { t } = useTranslation();
  if (records.length === 0) {
    return <span className="tool-empty-output">{emptyLabel}</span>;
  }
  return (
    <div className="mcp-catalog-list">
      {records.map((record) => (
        <div className="mcp-catalog-tool" key={`${record.server}/${record.tool}`}>
          <div className="mcp-catalog-tool-title">
            <code>
              <span>{record.server}</span> / <strong>{record.tool}</strong>
            </code>
            {record.read_only ? (
              <span className="mcp-catalog-read-only">{t("Read-only")}</span>
            ) : null}
          </div>
          {record.description ? (
            <p className="mcp-catalog-description">{record.description}</p>
          ) : null}
          <details className="mcp-catalog-schema">
            <summary>{t("Input schema")}</summary>
            <JsonTreeView data={record.input_schema} maxDepth={3} embedded />
          </details>
        </div>
      ))}
    </div>
  );
}

interface Props {
  catalog: McpCatalog;
  search: boolean;
}

export function McpCatalogRenderer({ catalog, search }: Props) {
  const { t } = useTranslation();
  const emptyLabel = search ? t("No matching MCP tools") : t("No MCP tools");
  const content = useMemo(() => {
    if (catalog.kind === "servers") return <ServerCatalog records={catalog.records} />;
    return <ToolCatalog records={catalog.records} emptyLabel={emptyLabel} />;
  }, [catalog, emptyLabel]);

  return <div className="mcp-catalog">{content}</div>;
}
