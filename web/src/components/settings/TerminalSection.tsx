import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useUIStore } from "../../stores/useUIStore";
import { detectSystemMonospaceFonts, type LocalFontDetectionStatus } from "../../terminal/fonts";
import {
  useTerminalShells,
  useTerminalShellConfig,
  useSetTerminalShellConfig,
} from "../../hooks/useQueries";
import { Button } from "../ui";
import {
  SettingsSectionHeader,
  SettingsGroup,
  SettingsSelectRow,
  SettingsInputRow,
  SettingsActionRow,
} from "./SettingsCommon";

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

  const shellDescription =
    localMode === "default"
      ? t("Uses the server's $SHELL environment variable (or /bin/bash as fallback).")
      : localMode === "selected"
        ? t("New terminal tabs will use {{shell}}.", { shell: terminalShell })
        : t("Enter the full path to your preferred shell executable.");

  const fontDetectionHelp =
    fontDetectionStatus === "loading"
      ? t("Detecting fonts...")
      : fontDetectionStatus === "unsupported"
        ? t("This browser cannot detect installed fonts. System default remains available.")
        : fontDetectionStatus === "denied"
          ? t("Font access was denied. Allow access and try again.")
          : fontDetectionStatus === "failed"
            ? t("Unable to detect installed fonts.")
            : fontDetectionStatus === "ready" && fontFamilies.length === 0
              ? t("No installed monospace fonts were found.")
              : t("Detect available monospace fonts on this browser device");

  return (
    <section className="space-y-6">
      <SettingsSectionHeader
        title={t("Terminal")}
        description={t("Choose which shell to use in the terminal")}
      />

      <SettingsGroup title={t("Shell Configuration")}>
        <SettingsSelectRow
          label={t("Shell")}
          description={
            loading ? (
              t("Loading shells...")
            ) : error ? (
              <span className="text-red-500">{error?.message ?? t("Failed to load shells")}</span>
            ) : (
              shellDescription
            )
          }
          value={selectValue}
          onValueChange={handleSelectChange}
          disabled={loading || Boolean(error)}
          options={[
            { value: "__default__", label: defaultShellLabel },
            ...(shellsData?.shells ?? []).map((shell) => ({
              value: shell.path,
              label: `${shell.name} (${shell.path})`,
            })),
            { value: "__custom__", label: t("Custom...") },
          ]}
        />
        {localMode === "custom" && (
          <SettingsInputRow
            label={t("Shell path or command")}
            value={customPath}
            onChange={handleCustomChange}
            placeholder="/usr/local/bin/nushell"
            mono
          />
        )}
      </SettingsGroup>

      <SettingsGroup title={t("Terminal Font")}>
        <SettingsSelectRow
          label={t("Primary terminal font")}
          description={t("The selected font is used first, with Unicode fallback fonts preserved.")}
          value={terminalFontFamily}
          onValueChange={(val) => updateSettings({ terminalFontFamily: val })}
          placeholder={t("System default")}
          options={[
            { value: "", label: t("System default") },
            ...(terminalFontFamily && !fontFamilies.includes(terminalFontFamily)
              ? [terminalFontFamily, ...fontFamilies]
              : fontFamilies
            ).map((family) => ({ value: family, label: family })),
          ]}
        />
        <SettingsActionRow
          label={t("Detect system fonts")}
          description={fontDetectionHelp}
          action={
            <Button
              size="sm"
              variant="secondary"
              loading={fontDetectionStatus === "loading"}
              onClick={handleDetectFonts}
            >
              {fontDetectionStatus === "loading"
                ? t("Detecting fonts...")
                : t("Detect system fonts")}
            </Button>
          }
        />
      </SettingsGroup>
    </section>
  );
}
