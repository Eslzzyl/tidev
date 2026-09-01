import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Terminal } from "lucide-react";
import { useUIStore } from "../../stores/useUIStore";
import { detectSystemMonospaceFonts, type LocalFontDetectionStatus } from "../../terminal/fonts";
import {
  useTerminalShells,
  useTerminalShellConfig,
  useSetTerminalShellConfig,
} from "../../hooks/useQueries";
import { Button, Input, Select } from "../ui";

type Mode = "default" | "selected" | "custom";

export function TerminalSection() {
  const { t } = useTranslation();
  const terminalShell = useUIStore((s) => s.settings.terminalShell);
  const terminalFontFamily = useUIStore((s) => s.settings.terminalFontFamily);
  const updateSettings = useUIStore((s) => s.updateSettings);
  const refreshTerminalFont = useUIStore((s) => s.refreshTerminalFont);

  const { data: shellsData, error, isLoading: loading } = useTerminalShells();
  const { data: configRes } = useTerminalShellConfig();
  const { mutateAsync: setTerminalShellConfig } = useSetTerminalShellConfig();

  const [localMode, setLocalMode] = useState<Mode>("default");
  const [customPath, setCustomPath] = useState("");
  const [fontFamilies, setFontFamilies] = useState<string[]>([]);
  const [fontDetectionStatus, setFontDetectionStatus] = useState<
    "idle" | "loading" | LocalFontDetectionStatus
  >("idle");

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

  const persistToServer = (shell: string) => {
    setTerminalShellConfig(shell).catch((err) => {
      console.warn("Failed to persist terminal shell config:", err);
    });
  };

  const handleDetectFonts = async () => {
    setFontDetectionStatus("loading");
    const result = await detectSystemMonospaceFonts();
    setFontFamilies(result.families);
    setFontDetectionStatus(result.status);
    if (result.status === "ready") refreshTerminalFont();
  };

  const handleSelectChange = (value: string) => {
    if (value === "__default__") {
      setLocalMode("default");
      updateSettings({ terminalShell: "" });
      persistToServer("");
    } else if (value === "__custom__") {
      setLocalMode("custom");
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

  const selectValue =
    localMode === "default" ? "__default__" : localMode === "custom" ? "__custom__" : terminalShell;

  const defaultShellLabel = shellsData
    ? t("System default ({{shell}})", { shell: shellsData.default_shell })
    : t("System default");

  return (
    <section className="space-y-6">
      <div>
        <h2 className="text-base font-semibold text-neutral-900 dark:text-neutral-100">
          {t("Terminal")}
        </h2>
        <p className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">
          {t("Choose which shell to use in the terminal")}
        </p>
      </div>

      <div className="space-y-3">
        <label className="text-xs font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
          {t("Shell Configuration")}
        </label>
        <div className="rounded-xl border border-neutral-200/80 bg-neutral-50/50 p-4 space-y-3 dark:border-neutral-800/80 dark:bg-neutral-800/30">
          <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
            <div className="flex items-center gap-3 min-w-0">
              <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-white shadow-xs text-neutral-600 dark:bg-neutral-800 dark:text-neutral-300">
                <Terminal className="h-4 w-4" />
              </div>
              <div className="min-w-0">
                <span className="block truncate text-xs font-medium text-neutral-900 dark:text-neutral-100">
                  {t("Shell")}
                </span>
                <span className="block text-[11px] text-neutral-500 dark:text-neutral-400">
                  {localMode === "default" &&
                    t("Uses the server's $SHELL environment variable (or /bin/bash as fallback).")}
                  {localMode === "selected" &&
                    t("New terminal tabs will use {{shell}}.", { shell: terminalShell })}
                  {localMode === "custom" &&
                    t("Enter the full path to your preferred shell executable.")}
                </span>
              </div>
            </div>

            <div className="shrink-0">
              {loading ? (
                <span className="text-xs text-neutral-500">{t("Loading shells...")}</span>
              ) : error ? (
                <span className="text-xs text-red-500">
                  {error?.message ?? t("Failed to load shells")}
                </span>
              ) : (
                <Select
                  value={selectValue}
                  onValueChange={handleSelectChange}
                  ariaLabel={t("Shell")}
                  className="terminal-shell-select min-w-[200px]"
                  options={[
                    { value: "__default__", label: defaultShellLabel },
                    ...(shellsData?.shells ?? []).map((shell) => ({
                      value: shell.path,
                      label: `${shell.name} (${shell.path})`,
                    })),
                    { value: "__custom__", label: t("Custom...") },
                  ]}
                />
              )}
            </div>
          </div>

          {localMode === "custom" && (
            <div className="pt-2 border-t border-neutral-200/60 dark:border-neutral-800/60">
              <label className="mb-1.5 block text-xs font-medium text-neutral-700 dark:text-neutral-300">
                {t("Shell path or command")}
              </label>
              <Input
                type="text"
                value={customPath}
                onChange={(e) => handleCustomChange(e.target.value)}
                placeholder="/usr/local/bin/nushell"
                className="font-mono"
              />
            </div>
          )}
        </div>
      </div>

      <div className="space-y-3">
        <label className="text-xs font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
          {t("Terminal Font")}
        </label>
        <div className="rounded-xl border border-neutral-200/80 bg-neutral-50/50 p-4 space-y-3 dark:border-neutral-800/80 dark:bg-neutral-800/30">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div className="min-w-0">
              <span className="block text-xs font-medium text-neutral-900 dark:text-neutral-100">
                {t("Primary terminal font")}
              </span>
              <span className="block text-[11px] text-neutral-500 dark:text-neutral-400">
                {t("The selected font is used first, with Unicode fallback fonts preserved.")}
              </span>
            </div>
            <Button
              size="sm"
              loading={fontDetectionStatus === "loading"}
              onClick={handleDetectFonts}
            >
              {fontDetectionStatus === "loading"
                ? t("Detecting fonts...")
                : t("Detect system fonts")}
            </Button>
          </div>

          <Select
            value={terminalFontFamily}
            onValueChange={(value) => updateSettings({ terminalFontFamily: value })}
            ariaLabel={t("Primary terminal font")}
            placeholder={t("System default")}
            options={[
              { value: "", label: t("System default") },
              ...(terminalFontFamily && !fontFamilies.includes(terminalFontFamily)
                ? [terminalFontFamily, ...fontFamilies]
                : fontFamilies
              ).map((family) => ({ value: family, label: family })),
            ]}
          />

          {fontDetectionStatus === "idle" && (
            <p className="text-[11px] text-neutral-500 dark:text-neutral-400">
              {t("Detect fonts to populate the list from this browser device.")}
            </p>
          )}
          {fontDetectionStatus === "unsupported" && (
            <p className="text-[11px] text-amber-600 dark:text-amber-400">
              {t("This browser cannot detect installed fonts. System default remains available.")}
            </p>
          )}
          {fontDetectionStatus === "denied" && (
            <p className="text-[11px] text-amber-600 dark:text-amber-400">
              {t("Font access was denied. Allow access and try again.")}
            </p>
          )}
          {fontDetectionStatus === "failed" && (
            <p className="text-[11px] text-red-600 dark:text-red-400">
              {t("Unable to detect installed fonts.")}
            </p>
          )}
          {fontDetectionStatus === "ready" && fontFamilies.length === 0 && (
            <p className="text-[11px] text-neutral-500 dark:text-neutral-400">
              {t("No installed monospace fonts were found.")}
            </p>
          )}
        </div>
      </div>
    </section>
  );
}
