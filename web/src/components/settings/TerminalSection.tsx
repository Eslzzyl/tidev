import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useUIStore } from "../../stores/useUIStore";
import {
  useTerminalShells,
  useTerminalShellConfig,
  useSetTerminalShellConfig,
} from "../../hooks/useQueries";

type Mode = "default" | "selected" | "custom";

export function TerminalSection() {
  const { t } = useTranslation();
  const terminalShell = useUIStore((s) => s.settings.terminalShell);
  const updateSettings = useUIStore((s) => s.updateSettings);

  const { data: shellsData, error, isLoading: loading } = useTerminalShells();
  const { data: configRes } = useTerminalShellConfig();
  const { mutateAsync: setTerminalShellConfig } = useSetTerminalShellConfig();

  // Local UI state
  const [localMode, setLocalMode] = useState<Mode>("default");
  const [customPath, setCustomPath] = useState("");

  // When both shells and config are loaded, apply server-side persisted config
  useEffect(() => {
    if (!shellsData || configRes === undefined) return;
    const stored = useUIStore.getState().settings.terminalShell;
    if (stored === "" && configRes.shell) {
      const envShell = shellsData.default_shell;
      if (configRes.shell !== envShell) {
        updateSettings({ terminalShell: configRes.shell });
      }
    }
  }, [shellsData, configRes, updateSettings]);

  // Sync local state with the stored value whenever shells become available
  useEffect(() => {
    if (!shellsData) return;
    const rafId = requestAnimationFrame(() => {
      if (terminalShell === "") {
        setLocalMode("default");
      } else if (shellsData.shells.some((s) => s.path === terminalShell)) {
        setLocalMode("selected");
      } else {
        setLocalMode("custom");
        setCustomPath(terminalShell);
      }
    });
    return () => cancelAnimationFrame(rafId);
  }, [terminalShell, shellsData]);

  // Persist to server-side config whenever the user explicitly changes shell
  const persistToServer = (shell: string) => {
    setTerminalShellConfig(shell).catch((err) => {
      console.warn("Failed to persist terminal shell config:", err);
    });
  };

  const handleSelectChange = (value: string) => {
    if (value === "__default__") {
      setLocalMode("default");
      updateSettings({ terminalShell: "" });
      persistToServer("");
    } else if (value === "__custom__") {
      setLocalMode("custom");
      // Don't update settings yet — the input field will handle it
    } else {
      setLocalMode("selected");
      updateSettings({ terminalShell: value });
      persistToServer(value);
    }
  };

  const handleCustomChange = (path: string) => {
    setCustomPath(path);
    updateSettings({ terminalShell: path });
    if (path) {
      persistToServer(path);
    }
  };

  // Determine the <select> value from localMode + stored value
  const selectValue =
    localMode === "default" ? "__default__" : localMode === "custom" ? "__custom__" : terminalShell;

  const defaultShellLabel = shellsData
    ? t("System default ({{shell}})", { shell: shellsData.default_shell })
    : t("System default");

  return (
    <section>
      <h2 className="mb-1 text-sm font-medium text-neutral-900 dark:text-neutral-100">
        {t("Terminal")}
      </h2>
      <p className="mb-4 text-sm text-neutral-500 dark:text-neutral-400">
        {t("Choose which shell to use in the terminal")}
      </p>

      <div className="flex flex-col gap-3 rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
        <label className="flex flex-col gap-2">
          <span className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
            {t("Shell")}
          </span>

          {loading ? (
            <span className="text-sm text-neutral-500">{t("Loading shells...")}</span>
          ) : error ? (
            <span className="text-sm text-red-500">
              {error?.message ?? t("Failed to load shells")}
            </span>
          ) : (
            <select
              value={selectValue}
              onChange={(e) => handleSelectChange(e.target.value)}
              className="w-full rounded-md border border-neutral-300 bg-white px-3 py-2 text-base text-neutral-900 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-neutral-600 dark:bg-neutral-800 dark:text-neutral-100 dark:focus:border-blue-400"
            >
              <option value="__default__">{defaultShellLabel}</option>
              {(shellsData?.shells ?? []).map((s) => (
                <option key={s.path} value={s.path}>
                  {s.name} ({s.path})
                </option>
              ))}
              <option value="__custom__">{t("Custom...")}</option>
            </select>
          )}
        </label>

        {/* Custom shell path input — always visible when in custom mode */}
        {localMode === "custom" && (
          <div>
            <label className="mb-1 block text-xs text-neutral-500 dark:text-neutral-400">
              {t("Shell path or command")}
            </label>
            <input
              type="text"
              value={customPath}
              onChange={(e) => handleCustomChange(e.target.value)}
              placeholder="/usr/local/bin/nushell"
              className="w-full rounded-md border border-neutral-300 bg-white px-3 py-2 text-base text-neutral-900 placeholder-neutral-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-neutral-600 dark:bg-neutral-800 dark:text-neutral-100 dark:placeholder-neutral-500 dark:focus:border-blue-400"
            />
          </div>
        )}

        {/* Hint text */}
        <p className="text-xs text-neutral-400 dark:text-neutral-500">
          {localMode === "default" &&
            t("Uses the server's $SHELL environment variable (or /bin/bash as fallback).")}
          {localMode === "selected" &&
            t("New terminal tabs will use {{shell}}.", { shell: terminalShell })}
          {localMode === "custom" && t("Enter the full path to your preferred shell executable.")}
        </p>
      </div>
    </section>
  );
}
