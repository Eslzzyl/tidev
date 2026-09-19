import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Terminal, RefreshCw } from "lucide-react";
import { api, waitForServerRestart } from "../../api/client";
import { useBuildInfo } from "../../hooks/workspaceQueries";
import { useUIStore } from "../../stores/useUIStore";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { Button } from "../ui";
import {
  SettingsSectionHeader,
  SettingsGroup,
  SettingsRow,
  SettingsActionRow,
} from "./SettingsCommon";

export function AboutSection() {
  const { t } = useTranslation();
  const connectionStatus = useUIStore((s) => s.connectionStatus);
  const [restarting, setRestarting] = useState(false);
  const [showConfirm, setShowConfirm] = useState(false);
  const { data: buildInfo } = useBuildInfo();

  const statusConfig: Record<string, { color: string; dot: string; label: string }> = {
    connected: {
      color:
        "text-emerald-700 dark:text-emerald-300 bg-emerald-50 dark:bg-emerald-950/50 border-emerald-200/60 dark:border-emerald-800/60",
      dot: "bg-emerald-500",
      label: t("Connected"),
    },
    disconnected: {
      color:
        "text-red-700 dark:text-red-300 bg-red-50 dark:bg-red-950/50 border-red-200/60 dark:border-red-900/60",
      dot: "bg-red-500",
      label: t("Disconnected"),
    },
    connecting: {
      color:
        "text-amber-700 dark:text-amber-300 bg-amber-50 dark:bg-amber-950/50 border-amber-200/60 dark:border-amber-900/60",
      dot: "bg-amber-500 animate-pulse",
      label: t("Connecting"),
    },
  };

  const currentStatus = statusConfig[connectionStatus] ?? statusConfig.disconnected;

  const confirmRestart = async () => {
    setShowConfirm(false);
    setRestarting(true);
    try {
      await api.restartServer();
      await waitForServerRestart();
      window.location.reload();
    } catch {
      setRestarting(false);
    }
  };

  return (
    <section className="space-y-6">
      <SettingsSectionHeader
        title={t("About")}
        description={t("Version, runtime status, and server maintenance")}
      />

      {/* Brand Header Card */}
      <div className="flex items-center gap-4 rounded-xl border border-neutral-200/80 bg-neutral-50/60 p-4 dark:border-neutral-800/80 dark:bg-neutral-800/40">
        <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-neutral-100 text-neutral-700 dark:bg-neutral-800 dark:text-neutral-300 shadow-xs">
          <Terminal className="h-6 w-6" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h3 className="text-base font-bold tracking-tight text-neutral-900 dark:text-neutral-100">
              tidev
            </h3>
            <span className="rounded-full bg-neutral-200/80 px-2 py-0.5 text-[11px] font-mono font-medium text-neutral-700 dark:bg-neutral-700 dark:text-neutral-300">
              {buildInfo ? `v${buildInfo.version}` : "—"}
            </span>
          </div>
          <p className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">
            {t("Agentic AI coding assistant")}
          </p>
        </div>
      </div>

      {/* System Status Details */}
      <SettingsGroup title={t("System Status")}>
        <SettingsRow
          label={t("Server Status")}
          control={
            <span
              className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium ${currentStatus.color}`}
            >
              <span className={`h-1.5 w-1.5 rounded-full ${currentStatus.dot}`} />
              {currentStatus.label}
            </span>
          }
        />
        <SettingsRow
          label={t("Web UI Version")}
          control={
            <span className="font-mono text-xs text-neutral-600 dark:text-neutral-400">
              {buildInfo?.version ?? "—"}
            </span>
          }
        />
      </SettingsGroup>

      {/* Server Management */}
      <SettingsGroup title={t("Server Management")}>
        <SettingsActionRow
          label={t("Restart Service")}
          description={t("Auto-reconnects after restart. Refresh manually if it does not recover.")}
          action={
            <Button
              type="button"
              disabled={restarting}
              onClick={() => setShowConfirm(true)}
              variant="secondary"
              size="sm"
              className="shrink-0 hover:border-red-300 hover:text-red-600 dark:hover:border-red-800 dark:hover:text-red-400"
              leadingIcon={
                <RefreshCw className={`h-3.5 w-3.5 ${restarting ? "animate-spin" : ""}`} />
              }
            >
              {restarting ? t("Restarting...") : t("Restart Service")}
            </Button>
          }
        />
      </SettingsGroup>

      <ConfirmDialog
        isOpen={showConfirm}
        title={t("Restart server")}
        message={t(
          "Are you sure you want to restart the tidev server? The frontend will automatically reconnect once the server is ready.",
        )}
        confirmText={t("Restart")}
        danger
        onConfirm={confirmRestart}
        onCancel={() => setShowConfirm(false)}
      />
    </section>
  );
}
