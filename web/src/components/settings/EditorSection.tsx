import { useTranslation } from "react-i18next";
import { useUIStore } from "../../stores/useUIStore";
import {
  SettingsSectionHeader,
  SettingsGroup,
  SettingsInputRow,
  SettingsSelectRow,
} from "./SettingsCommon";

const FONT_SIZE_OPTIONS = [12, 13, 14, 15, 16, 18, 20].map((size) => ({
  value: String(size),
  label: `${size}px`,
}));

export function EditorSection() {
  const { t } = useTranslation();
  const settings = useUIStore((s) => s.settings);
  const updateSettings = useUIStore((s) => s.updateSettings);

  return (
    <section className="space-y-6">
      <SettingsSectionHeader
        title={t("Editor")}
        description={t("Customize the display fonts and code diff layout")}
      />

      <SettingsGroup title={t("Typography")}>
        <SettingsInputRow
          label={t("UI Font")}
          description={t("Font family for the user interface")}
          value={settings.fontFamily}
          onChange={(val) => updateSettings({ fontFamily: val })}
          placeholder="Inter, system-ui, sans-serif"
        />
        <SettingsInputRow
          label={t("Monospace Font")}
          description={t("Font family for code blocks and diffs")}
          value={settings.monoFontFamily}
          onChange={(val) => updateSettings({ monoFontFamily: val })}
          placeholder="JetBrains Mono, Fira Code, monospace"
          mono
        />
        <SettingsSelectRow
          label={t("Base font size")}
          description={t("Base font size for editor and code blocks")}
          value={String(settings.fontSize)}
          onValueChange={(val) => updateSettings({ fontSize: Number(val) })}
          options={FONT_SIZE_OPTIONS}
        />
      </SettingsGroup>

      <SettingsGroup title={t("Code Diff Layout")}>
        <SettingsSelectRow
          label={t("Diff Layout")}
          description={t("Choose how file differences are displayed")}
          value={settings.diffLayout}
          onValueChange={(val) => updateSettings({ diffLayout: val as "side-by-side" | "inline" })}
          options={[
            { value: "side-by-side", label: t("Side by Side") },
            { value: "inline", label: t("Inline") },
          ]}
        />
      </SettingsGroup>
    </section>
  );
}
