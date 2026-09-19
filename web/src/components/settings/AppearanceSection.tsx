import { useTranslation } from "react-i18next";
import { useUIStore, type Theme } from "../../stores/useUIStore";
import type { LocalePreference } from "../../i18n";
import { SettingsSectionHeader, SettingsGroup, SettingsSelectRow } from "./SettingsCommon";

const THEME_OPTIONS: { value: Theme; labelKey: string }[] = [
  { value: "system", labelKey: "System" },
  { value: "light", labelKey: "Light" },
  { value: "dark", labelKey: "Dark" },
];

export function AppearanceSection() {
  const { t } = useTranslation();
  const theme = useUIStore((s) => s.theme);
  const locale = useUIStore((s) => s.locale);
  const setTheme = useUIStore((s) => s.setTheme);
  const setLocale = useUIStore((s) => s.setLocale);

  return (
    <section className="space-y-6">
      <SettingsSectionHeader
        title={t("Appearance")}
        description={t("Choose your preferred color theme")}
      />

      <SettingsGroup title={t("Theme")}>
        <SettingsSelectRow
          label={t("Theme mode")}
          description={t("Choose your preferred color theme (Light, Dark, or System)")}
          value={theme}
          onValueChange={(val) => setTheme(val as Theme)}
          options={THEME_OPTIONS.map((opt) => ({
            value: opt.value,
            label: t(opt.labelKey),
          }))}
        />
        <SettingsSelectRow
          label={t("Display language")}
          description={t("Choose the display language for the application")}
          value={locale}
          onValueChange={(val) => setLocale(val as LocalePreference)}
          options={[
            { value: "system", label: t("Use browser language") },
            { value: "zh-CN", label: t("简体中文") },
            { value: "en", label: t("English") },
          ]}
        />
      </SettingsGroup>
    </section>
  );
}
