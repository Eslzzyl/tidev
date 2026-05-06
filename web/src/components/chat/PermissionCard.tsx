import { useState } from "react";
import {
  ShieldAlert,
  Check,
  X,
  Clock,
  FileEdit,
  Terminal,
  Globe,
  Wrench,
} from "lucide-react";
import { usePermissionStore, type PendingPermission } from "../../stores/usePermissionStore";

interface PermissionCardProps {
  permission: PendingPermission;
  onResponse: (response: "once" | "always" | "deny") => void;
}

function getToolIcon(toolName: string) {
  const name = toolName.toLowerCase();
  if (["write", "edit", "apply_patch", "str_replace"].includes(name))
    return <FileEdit className="h-4 w-4" />;
  if (["bash", "shell", "cmd", "terminal"].includes(name))
    return <Terminal className="h-4 w-4" />;
  if (["webfetch", "fetch", "websearch"].includes(name))
    return <Globe className="h-4 w-4" />;
  return <Wrench className="h-4 w-4" />;
}

function getToolColor(toolName: string): string {
  const name = toolName.toLowerCase();
  if (["write", "edit", "apply_patch", "str_replace"].includes(name))
    return "text-emerald-600 dark:text-emerald-400";
  if (["bash", "shell", "cmd", "terminal"].includes(name))
    return "text-violet-600 dark:text-violet-400";
  if (["webfetch", "fetch", "websearch"].includes(name))
    return "text-sky-600 dark:text-sky-400";
  return "text-neutral-600 dark:text-neutral-400";
}

function formatArguments(args: string): string {
  try {
    const parsed = JSON.parse(args);
    if (typeof parsed === "object" && parsed !== null) {
      // Show command or key argument
      const cmd = parsed.command || parsed.path || parsed.url || parsed.query;
      if (cmd) return String(cmd);
      // Otherwise format as compact JSON
      const entries = Object.entries(parsed).slice(0, 3);
      const str = entries.map(([k, v]) => `${k}=${typeof v === "string" ? v : JSON.stringify(v)}`).join(", ");
      return entries.length < Object.keys(parsed).length ? `${str}...` : str;
    }
    return args;
  } catch {
    return args;
  }
}

function getDisplayName(toolName: string): string {
  const name = toolName.toLowerCase();
  if (["write", "file_write"].includes(name)) return "Write File";
  if (["edit", "str_replace", "str_replace_based_edit_tool"].includes(name)) return "Edit File";
  if (["apply_patch"].includes(name)) return "Apply Patch";
  if (["bash", "shell", "cmd", "terminal"].includes(name)) return "Bash";
  if (["webfetch", "fetch", "websearch"].includes(name)) return "Web Request";
  return toolName;
}

export function PermissionCard({ permission, onResponse }: PermissionCardProps) {
  const [isResponding, setIsResponding] = useState(false);
  const removePermission = usePermissionStore((s) => s.removePermission);
  const setAutoAccept = usePermissionStore((s) => s.setAutoAccept);

  const handleResponse = (response: "once" | "always" | "deny") => {
    setIsResponding(true);
    if (response === "always") {
      setAutoAccept(permission.sessionId, true);
    }
    onResponse(response);
    removePermission(permission.id);
  };

  return (
    <div className="my-3 overflow-hidden rounded-lg border border-amber-200 bg-amber-50 dark:border-amber-800 dark:bg-amber-950/20">
      {/* Header */}
      <div className="flex items-center gap-2 border-b border-amber-200/60 px-3 py-2 dark:border-amber-800/60">
        <ShieldAlert className="h-4 w-4 text-amber-600 dark:text-amber-400" />
        <span className="text-xs font-semibold text-amber-800 dark:text-amber-300">
          Permission Request
        </span>
      </div>

      {/* Tool info */}
      <div className="px-3 py-2">
        <div className="flex items-center gap-2">
          <span className={getToolColor(permission.toolName)}>
            {getToolIcon(permission.toolName)}
          </span>
          <span className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
            {getDisplayName(permission.toolName)}
          </span>
        </div>

        <div className="mt-1.5 overflow-x-auto rounded bg-amber-100/60 px-2.5 py-1.5 font-mono text-xs leading-relaxed text-neutral-700 dark:bg-amber-900/30 dark:text-neutral-300">
          {formatArguments(permission.arguments)}
        </div>
      </div>

      {/* Actions */}
      <div className="flex items-center gap-1.5 border-t border-amber-200/60 px-3 py-2 dark:border-amber-800/60">
        <button
          onClick={() => handleResponse("once")}
          disabled={isResponding}
          className="flex items-center gap-1.5 rounded bg-amber-600 px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-amber-700 disabled:opacity-50"
        >
          <Check className="h-3 w-3" />
          Allow once
        </button>
        <button
          onClick={() => handleResponse("always")}
          disabled={isResponding}
          className="flex items-center gap-1.5 rounded border border-amber-300 bg-white px-3 py-1.5 text-xs font-medium text-amber-700 transition-colors hover:bg-amber-100 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-300 dark:hover:bg-amber-900/50 disabled:opacity-50"
        >
          <Clock className="h-3 w-3" />
          Always allow
        </button>
        <button
          onClick={() => handleResponse("deny")}
          disabled={isResponding}
          className="ml-auto flex items-center gap-1.5 rounded px-3 py-1.5 text-xs font-medium text-neutral-600 transition-colors hover:bg-amber-100/50 dark:text-neutral-400 dark:hover:bg-amber-900/30 disabled:opacity-50"
        >
          <X className="h-3 w-3" />
          Deny
        </button>
      </div>
    </div>
  );
}
