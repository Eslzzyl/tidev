import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Boxes,
  Plus,
  RefreshCw,
  Trash2,
  Edit2,
  Power,
  ChevronDown,
  ChevronRight,
  AlertCircle,
  Terminal,
  Globe,
  Radio,
  X,
} from "lucide-react";
import {
  useMcpServers,
  useUpsertMcpServer,
  useDeleteMcpServer,
  useConnectMcpServer,
  useDisconnectMcpServer,
  useRefreshMcpServer,
} from "../../hooks/workspaceQueries";
import type { McpServerInfo, McpServerConfig, McpToolSummary } from "../../types/api";

interface ServerDraft {
  name: string;
  type: "stdio" | "http" | "sse";
  command: string;
  args: string;
  cwd: string;
  env: string;
  url: string;
  headers: string;
}

const DEFAULT_DRAFT: ServerDraft = {
  name: "",
  type: "stdio",
  command: "",
  args: "",
  cwd: "",
  env: "",
  url: "",
  headers: "",
};

function serverToDraft(server: McpServerInfo): ServerDraft {
  const draft: ServerDraft = {
    name: server.name,
    type: (server.kind as "stdio" | "http" | "sse") || "stdio",
    command: "",
    args: "",
    cwd: "",
    env: "",
    url: "",
    headers: "",
  };

  if (server.config) {
    const cfg = server.config as Record<string, unknown>;
    const isStdio = server.kind === "stdio" || "command" in cfg;
    if (isStdio) {
      draft.type = "stdio";
      draft.command = typeof cfg.command === "string" ? cfg.command : "";
      draft.args = Array.isArray(cfg.args) ? (cfg.args as string[]).join(" ") : "";
      draft.cwd = typeof cfg.cwd === "string" ? cfg.cwd : "";
      draft.env =
        cfg.env && typeof cfg.env === "object"
          ? Object.entries(cfg.env as Record<string, string>)
              .map(([k, v]) => `${k}=${v}`)
              .join("\n")
          : "";
    } else {
      draft.type = (server.kind as "http" | "sse") || (cfg.type as "http" | "sse") || "http";
      draft.url = typeof cfg.url === "string" ? cfg.url : "";
      draft.headers =
        cfg.headers && typeof cfg.headers === "object"
          ? Object.entries(cfg.headers as Record<string, string>)
              .map(([k, v]) => `${k}: ${v}`)
              .join("\n")
          : "";
    }
  }

  return draft;
}

function serverDetailLine(server: McpServerInfo, t: (key: string) => string): string {
  if (!server.config) return t("No configuration");
  const cfg = server.config as Record<string, unknown>;
  const isStdio = server.kind === "stdio" || "command" in cfg;
  if (isStdio) {
    const command = typeof cfg.command === "string" ? cfg.command : "";
    const args = Array.isArray(cfg.args) ? (cfg.args as string[]).join(" ") : "";
    const combined = `${command} ${args}`.trim();
    return combined || t("No command");
  }
  const url = typeof cfg.url === "string" ? cfg.url : "";
  return url || t("No URL");
}

function draftToConfig(draft: ServerDraft): { config: McpServerConfig; error?: string } {
  if (!draft.name.trim()) {
    return { config: { type: "stdio", command: "" }, error: "Server name is required" };
  }

  if (draft.type === "stdio") {
    if (!draft.command.trim()) {
      return {
        config: { type: "stdio", command: "" },
        error: "Command is required for stdio server",
      };
    }
    const args = draft.args.trim().split(/\s+/).filter(Boolean);
    const env: Record<string, string> = {};
    for (const line of draft.env.split("\n")) {
      const trimmed = line.trim();
      if (!trimmed || trimmed.startsWith("#")) continue;
      const idx = trimmed.indexOf("=");
      if (idx > 0) {
        env[trimmed.slice(0, idx).trim()] = trimmed.slice(idx + 1).trim();
      }
    }
    return {
      config: {
        type: "stdio",
        command: draft.command.trim(),
        args,
        cwd: draft.cwd.trim() || null,
        env,
      },
    };
  }

  if (!draft.url.trim()) {
    return { config: { type: "http", url: "" }, error: "URL is required" };
  }

  const headers: Record<string, string> = {};
  for (const line of draft.headers.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const idx = trimmed.indexOf(":");
    if (idx > 0) {
      headers[trimmed.slice(0, idx).trim()] = trimmed.slice(idx + 1).trim();
    }
  }

  return {
    config: {
      type: draft.type,
      url: draft.url.trim(),
      headers,
    },
  };
}

export function McpSection() {
  const { t } = useTranslation();
  const { data: servers = [], isLoading, error } = useMcpServers();
  const { mutateAsync: upsertServer, isPending: isUpserting } = useUpsertMcpServer();
  const { mutateAsync: deleteServer } = useDeleteMcpServer();
  const { mutateAsync: connectServer } = useConnectMcpServer();
  const { mutateAsync: disconnectServer } = useDisconnectMcpServer();
  const { mutateAsync: refreshServer } = useRefreshMcpServer();

  const [expandedServer, setExpandedServer] = useState<string | null>(null);
  const [modalOpen, setModalOpen] = useState(false);
  const [editingServerName, setEditingServerName] = useState<string | null>(null);
  const [draft, setDraft] = useState<ServerDraft>(DEFAULT_DRAFT);
  const [formError, setFormError] = useState<string | null>(null);
  const [actionInProgress, setActionInProgress] = useState<string | null>(null);

  const handleOpenAdd = () => {
    setEditingServerName(null);
    setDraft(DEFAULT_DRAFT);
    setFormError(null);
    setModalOpen(true);
  };

  const handleOpenEdit = (server: McpServerInfo) => {
    setEditingServerName(server.name);
    setDraft(serverToDraft(server));
    setFormError(null);
    setModalOpen(true);
  };

  const handleSaveModal = async () => {
    const { config, error } = draftToConfig(draft);
    if (error) {
      setFormError(error);
      return;
    }

    try {
      await upsertServer({
        name: draft.name.trim(),
        config,
        original_name: editingServerName ?? undefined,
      });
      setModalOpen(false);
    } catch (err: unknown) {
      setFormError(err instanceof Error ? err.message : "Failed to save server");
    }
  };

  const handleDelete = async (name: string) => {
    if (window.confirm(t('Are you sure you want to remove MCP server "{{name}}"?', { name }))) {
      try {
        await deleteServer(name);
      } catch (err) {
        console.error("Failed to delete MCP server:", err);
      }
    }
  };

  const handleToggleConnection = async (server: McpServerInfo) => {
    setActionInProgress(server.name);
    try {
      if (server.status === "connected" || server.status === "connecting") {
        await disconnectServer(server.name);
      } else {
        await connectServer(server.name);
      }
    } catch (err) {
      console.error("Failed to toggle MCP server connection:", err);
    } finally {
      setActionInProgress(null);
    }
  };

  const handleRefresh = async (name: string) => {
    setActionInProgress(name);
    try {
      await refreshServer(name);
    } catch (err) {
      console.error("Failed to refresh MCP server tools:", err);
    } finally {
      setActionInProgress(null);
    }
  };

  const toggleExpand = (name: string) => {
    setExpandedServer((prev) => (prev === name ? null : name));
  };

  return (
    <section className="space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
            {t("MCP Servers")}
          </h2>
          <p className="text-xs text-neutral-500 dark:text-neutral-400">
            {t("Manage Model Context Protocol (MCP) server connections and tools")}
          </p>
        </div>
        <button
          onClick={handleOpenAdd}
          className="flex items-center gap-1.5 rounded-lg bg-neutral-900 px-3 py-1.5 text-xs font-medium !text-white transition hover:bg-neutral-800 dark:bg-neutral-100 dark:!text-neutral-900 dark:hover:bg-neutral-200"
        >
          <Plus className="h-3.5 w-3.5" />
          <span>{t("Add Server")}</span>
        </button>
      </div>

      {/* Loading & Error States */}
      {isLoading ? (
        <div className="flex items-center justify-center py-10 text-neutral-500">
          <RefreshCw className="h-5 w-5 animate-spin mr-2" />
          <span className="text-sm">{t("Loading MCP servers...")}</span>
        </div>
      ) : error ? (
        <div className="rounded-lg border border-red-200 bg-red-50 p-3 text-xs text-red-700 dark:border-red-900/50 dark:bg-red-950/30 dark:text-red-400">
          {t("Failed to load MCP servers")}
        </div>
      ) : servers.length === 0 ? (
        <div className="flex flex-col items-center justify-center rounded-xl border border-dashed border-neutral-300 py-12 text-center dark:border-neutral-800">
          <Boxes className="h-10 w-10 text-neutral-400 mb-2" />
          <p className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
            {t("No MCP servers configured")}
          </p>
          <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-1 max-w-sm">
            {t("Add stdio, HTTP, or SSE MCP servers to extend your assistant with custom tools.")}
          </p>
          <button
            onClick={handleOpenAdd}
            className="mt-4 flex items-center gap-1.5 rounded-lg bg-neutral-900 px-3 py-1.5 text-xs font-medium !text-white transition hover:bg-neutral-800 dark:bg-neutral-100 dark:!text-neutral-900 dark:hover:bg-neutral-200"
          >
            <Plus className="h-3.5 w-3.5" />
            <span>{t("Add your first server")}</span>
          </button>
        </div>
      ) : (
        /* Server List */
        <div className="space-y-3">
          {servers.map((server) => {
            const isExpanded = expandedServer === server.name;
            const isBusy = actionInProgress === server.name;
            const isConnected = server.status === "connected";
            const isConnecting = server.status === "connecting";
            const isFailed = server.status === "failed";

            return (
              <div
                key={server.name}
                className="overflow-hidden rounded-xl border border-neutral-200 bg-white transition shadow-sm dark:border-neutral-800 dark:bg-neutral-900"
              >
                {/* Server Row */}
                <div className="flex items-center justify-between p-3.5">
                  <div className="flex items-center gap-3 min-w-0">
                    {/* Status Dot */}
                    <div className="relative flex items-center justify-center">
                      <span
                        className={`h-2.5 w-2.5 rounded-full ${
                          isConnected
                            ? "bg-emerald-500"
                            : isConnecting
                              ? "bg-amber-500 animate-pulse"
                              : isFailed
                                ? "bg-red-500"
                                : "bg-neutral-400"
                        }`}
                      />
                    </div>

                    {/* Server Info */}
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="font-semibold text-sm text-neutral-900 dark:text-neutral-100 truncate">
                          {server.name}
                        </span>
                        <span className="inline-flex items-center rounded-md bg-neutral-100 px-1.5 py-0.5 text-[10px] font-medium text-neutral-600 dark:bg-neutral-800 dark:text-neutral-400">
                          {server.kind === "stdio" ? (
                            <Terminal className="h-2.5 w-2.5 mr-1" />
                          ) : server.kind === "http" ? (
                            <Globe className="h-2.5 w-2.5 mr-1" />
                          ) : (
                            <Radio className="h-2.5 w-2.5 mr-1" />
                          )}
                          {server.kind}
                        </span>
                        <span
                          className={`text-[11px] font-medium ${
                            isConnected
                              ? "text-emerald-600 dark:text-emerald-400"
                              : isConnecting
                                ? "text-amber-600 dark:text-amber-400"
                                : isFailed
                                  ? "text-red-600 dark:text-red-400"
                                  : "text-neutral-500"
                          }`}
                        >
                          {isConnected
                            ? t("Connected")
                            : isConnecting
                              ? t("Connecting...")
                              : isFailed
                                ? t("Failed")
                                : t("Disconnected")}
                        </span>
                      </div>

                      {/* Detail Line */}
                      <p className="text-xs text-neutral-500 dark:text-neutral-400 truncate mt-0.5 font-mono">
                        {serverDetailLine(server, t)}
                      </p>
                    </div>
                  </div>

                  {/* Action Buttons */}
                  <div className="flex items-center gap-1.5 shrink-0">
                    {/* Toggle Connect/Disconnect */}
                    <button
                      onClick={() => handleToggleConnection(server)}
                      disabled={isBusy}
                      title={isConnected ? t("Disconnect") : t("Connect")}
                      className={`p-1.5 rounded-lg border transition ${
                        isConnected
                          ? "border-emerald-200 text-emerald-600 hover:bg-emerald-50 dark:border-emerald-900/50 dark:text-emerald-400 dark:hover:bg-emerald-950/30"
                          : "border-neutral-200 text-neutral-600 hover:bg-neutral-100 dark:border-neutral-700 dark:text-neutral-400 dark:hover:bg-neutral-800"
                      }`}
                    >
                      <Power className={`h-4 w-4 ${isBusy ? "animate-spin" : ""}`} />
                    </button>

                    {/* Refresh Tools */}
                    <button
                      onClick={() => handleRefresh(server.name)}
                      disabled={isBusy}
                      title={t("Refresh Tools")}
                      className="p-1.5 rounded-lg border border-neutral-200 text-neutral-600 hover:bg-neutral-100 dark:border-neutral-700 dark:text-neutral-400 dark:hover:bg-neutral-800 transition"
                    >
                      <RefreshCw className={`h-4 w-4 ${isBusy ? "animate-spin" : ""}`} />
                    </button>

                    {/* Edit */}
                    <button
                      onClick={() => handleOpenEdit(server)}
                      title={t("Edit Server")}
                      className="p-1.5 rounded-lg border border-neutral-200 text-neutral-600 hover:bg-neutral-100 dark:border-neutral-700 dark:text-neutral-400 dark:hover:bg-neutral-800 transition"
                    >
                      <Edit2 className="h-4 w-4" />
                    </button>

                    {/* Delete */}
                    <button
                      onClick={() => handleDelete(server.name)}
                      title={t("Remove Server")}
                      className="p-1.5 rounded-lg border border-neutral-200 text-red-600 hover:bg-red-50 dark:border-neutral-700 dark:text-red-400 dark:hover:bg-red-950/30 transition"
                    >
                      <Trash2 className="h-4 w-4" />
                    </button>

                    {/* Expand Tools Toggle */}
                    <button
                      onClick={() => toggleExpand(server.name)}
                      title={isExpanded ? t("Collapse tools") : t("Expand tools")}
                      className="p-1.5 text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200 transition"
                    >
                      {isExpanded ? (
                        <ChevronDown className="h-4 w-4" />
                      ) : (
                        <ChevronRight className="h-4 w-4" />
                      )}
                    </button>
                  </div>
                </div>

                {/* Error Banner if any */}
                {server.error && (
                  <div className="bg-red-50 px-3.5 py-2 border-t border-red-100 text-xs text-red-700 dark:bg-red-950/40 dark:border-red-900/40 dark:text-red-300 flex items-start gap-2">
                    <AlertCircle className="h-4 w-4 shrink-0 mt-0.5" />
                    <span className="break-all">{server.error}</span>
                  </div>
                )}

                {/* Tools Accordion Body */}
                {isExpanded && (
                  <div className="border-t border-neutral-200 bg-neutral-50/50 p-3.5 dark:border-neutral-800 dark:bg-neutral-900/50">
                    <div className="flex items-center justify-between mb-2">
                      <span className="text-xs font-semibold text-neutral-700 dark:text-neutral-300">
                        {t("Registered Tools ({{count}})", { count: server.tools.length })}
                      </span>
                    </div>

                    {server.tools.length === 0 ? (
                      <p className="text-xs text-neutral-400 italic">
                        {isConnected
                          ? t("No tools advertised by this server.")
                          : t("Connect to discover available tools.")}
                      </p>
                    ) : (
                      <div className="grid grid-cols-1 gap-2">
                        {server.tools.map((tool: McpToolSummary) => (
                          <div
                            key={tool.name}
                            className="rounded-lg border border-neutral-200 bg-white p-2.5 dark:border-neutral-800 dark:bg-neutral-800/60"
                          >
                            <div className="flex items-center gap-2">
                              <span className="font-mono text-xs font-semibold text-neutral-900 dark:text-neutral-100">
                                {tool.name}
                              </span>
                            </div>
                            {tool.description && (
                              <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-1">
                                {tool.description}
                              </p>
                            )}
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}

      {/* Add / Edit Server Modal */}
      {modalOpen && (
        <div className="fixed inset-0 z-70 flex items-center justify-center bg-black/50 p-4">
          <div className="flex max-h-[90vh] w-full max-w-lg flex-col overflow-hidden rounded-xl bg-white shadow-2xl dark:bg-neutral-900">
            {/* Modal Header */}
            <div className="flex items-center justify-between border-b border-neutral-200 px-5 py-3.5 dark:border-neutral-800">
              <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
                {editingServerName ? t("Edit MCP Server") : t("Add MCP Server")}
              </h3>
              <button
                onClick={() => setModalOpen(false)}
                className="rounded p-1 text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            {/* Modal Form */}
            <div className="flex-1 overflow-y-auto p-5 space-y-4">
              {formError && (
                <div className="rounded-lg bg-red-50 p-2.5 text-xs text-red-700 dark:bg-red-950/40 dark:text-red-300">
                  {formError}
                </div>
              )}

              {/* Server Name */}
              <div>
                <label className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1">
                  {t("Server Name")}
                </label>
                <input
                  type="text"
                  value={draft.name}
                  onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                  placeholder="e.g. blender, github, memory"
                  className="w-full rounded-lg border border-neutral-300 px-3 py-1.5 text-xs text-neutral-900 focus:border-neutral-500 focus:outline-none dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
                />
              </div>

              {/* Transport Type */}
              <div>
                <label className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1">
                  {t("Transport Type")}
                </label>
                <div className="grid grid-cols-3 gap-2">
                  {(["stdio", "http", "sse"] as const).map((type) => (
                    <button
                      key={type}
                      type="button"
                      onClick={() => setDraft({ ...draft, type })}
                      className={`flex items-center justify-center gap-1.5 rounded-lg border py-2 text-xs font-medium transition ${
                        draft.type === type
                          ? "border-neutral-900 bg-neutral-900 !text-white dark:border-neutral-100 dark:bg-neutral-100 dark:!text-neutral-900"
                          : "border-neutral-200 text-neutral-600 hover:bg-neutral-50 dark:border-neutral-700 dark:text-neutral-400 dark:hover:bg-neutral-800"
                      }`}
                    >
                      {type === "stdio" ? (
                        <Terminal className="h-3 w-3" />
                      ) : type === "http" ? (
                        <Globe className="h-3 w-3" />
                      ) : (
                        <Radio className="h-3 w-3" />
                      )}
                      {type.toUpperCase()}
                    </button>
                  ))}
                </div>
              </div>

              {/* Fields for stdio */}
              {draft.type === "stdio" ? (
                <>
                  <div>
                    <label className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1">
                      {t("Command")}
                    </label>
                    <input
                      type="text"
                      value={draft.command}
                      onChange={(e) => setDraft({ ...draft, command: e.target.value })}
                      placeholder="e.g. uvx, npx, python, /path/to/server"
                      className="w-full rounded-lg border border-neutral-300 px-3 py-1.5 text-xs text-neutral-900 focus:border-neutral-500 focus:outline-none dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100 font-mono"
                    />
                  </div>

                  <div>
                    <label className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1">
                      {t("Arguments (space separated)")}
                    </label>
                    <input
                      type="text"
                      value={draft.args}
                      onChange={(e) => setDraft({ ...draft, args: e.target.value })}
                      placeholder="e.g. blender-mcp or -y @modelcontextprotocol/server-filesystem"
                      className="w-full rounded-lg border border-neutral-300 px-3 py-1.5 text-xs text-neutral-900 focus:border-neutral-500 focus:outline-none dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100 font-mono"
                    />
                  </div>

                  <div>
                    <label className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1">
                      {t("Working Directory (optional)")}
                    </label>
                    <input
                      type="text"
                      value={draft.cwd}
                      onChange={(e) => setDraft({ ...draft, cwd: e.target.value })}
                      placeholder={t("Leave empty for workspace root")}
                      className="w-full rounded-lg border border-neutral-300 px-3 py-1.5 text-xs text-neutral-900 focus:border-neutral-500 focus:outline-none dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
                    />
                  </div>

                  <div>
                    <label className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1">
                      {t("Environment Variables (KEY=VALUE per line)")}
                    </label>
                    <textarea
                      rows={3}
                      value={draft.env}
                      onChange={(e) => setDraft({ ...draft, env: e.target.value })}
                      placeholder="API_KEY=xyz&#10;DEBUG=1"
                      className="w-full rounded-lg border border-neutral-300 px-3 py-1.5 text-xs text-neutral-900 focus:border-neutral-500 focus:outline-none dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100 font-mono"
                    />
                  </div>
                </>
              ) : (
                /* Fields for http/sse */
                <>
                  <div>
                    <label className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1">
                      {t("Endpoint URL")}
                    </label>
                    <input
                      type="text"
                      value={draft.url}
                      onChange={(e) => setDraft({ ...draft, url: e.target.value })}
                      placeholder="http://127.0.0.1:8000/mcp"
                      className="w-full rounded-lg border border-neutral-300 px-3 py-1.5 text-xs text-neutral-900 focus:border-neutral-500 focus:outline-none dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100 font-mono"
                    />
                  </div>

                  <div>
                    <label className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1">
                      {t("Headers (Header: Value per line)")}
                    </label>
                    <textarea
                      rows={3}
                      value={draft.headers}
                      onChange={(e) => setDraft({ ...draft, headers: e.target.value })}
                      placeholder="Authorization: Bearer token&#10;X-Custom-Header: value"
                      className="w-full rounded-lg border border-neutral-300 px-3 py-1.5 text-xs text-neutral-900 focus:border-neutral-500 focus:outline-none dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100 font-mono"
                    />
                  </div>
                </>
              )}
            </div>

            {/* Modal Footer */}
            <div className="flex items-center justify-end gap-2 border-t border-neutral-200 px-5 py-3 dark:border-neutral-800">
              <button
                type="button"
                onClick={() => setModalOpen(false)}
                className="rounded-lg border border-neutral-300 px-3 py-1.5 text-xs font-medium text-neutral-700 hover:bg-neutral-50 dark:border-neutral-700 dark:text-neutral-300 dark:hover:bg-neutral-800"
              >
                {t("Cancel")}
              </button>
              <button
                type="button"
                onClick={handleSaveModal}
                disabled={isUpserting}
                className="flex items-center gap-1.5 rounded-lg bg-neutral-900 px-4 py-1.5 text-xs font-medium !text-white transition hover:bg-neutral-800 disabled:opacity-50 dark:bg-neutral-100 dark:!text-neutral-900 dark:hover:bg-neutral-200"
              >
                {isUpserting ? <RefreshCw className="h-3.5 w-3.5 animate-spin" /> : null}
                <span>{t("Save Server")}</span>
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
