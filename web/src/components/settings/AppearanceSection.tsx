import { Sun, Moon, Monitor } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useUIStore, getEffectiveTheme, type Theme } from "../../stores/useUIStore";
import type { LocalePreference } from "../../i18n";
import { Button, Select } from "../ui";

const themes: { value: Theme; label: string; icon: React.ReactNode }[] = [
  {
    value: "light",
    label: "Light",
    icon: <Sun className="h-8 w-8 text-neutral-900 dark:text-neutral-100" />,
  },
  {
    value: "dark",
    label: "Dark",
    icon: <Moon className="h-8 w-8 text-neutral-900 dark:text-neutral-100" />,
  },
  {
    value: "system",
    label: "System",
    icon: <Monitor className="h-8 w-8 text-neutral-900 dark:text-neutral-100" />,
  },
];

export function AppearanceSection() {
  const { t } = useTranslation();
  const theme = useUIStore((s) => s.theme);
  const locale = useUIStore((s) => s.locale);
  const setTheme = useUIStore((s) => s.setTheme);
  const setLocale = useUIStore((s) => s.setLocale);
  const effectiveTheme = getEffectiveTheme(theme);

  return (
    <section>
      <h2 className="mb-1 text-sm font-medium text-neutral-900 dark:text-neutral-100">
        {t("Appearance")}
      </h2>
      <p className="mb-4 text-sm text-neutral-500 dark:text-neutral-400">
        {t("Choose your preferred color theme")}
      </p>

      <div className="grid grid-cols-3 gap-3">
        {themes.map((themeOption) => (
          <Button
            type="button"
            key={themeOption.value}
            onClick={() => setTheme(themeOption.value)}
            className="theme-option-button"
            variant="secondary"
            size="lg"
            data-active={themeOption.value === theme ? "true" : undefined}
            leadingIcon={themeOption.icon}
          >
            <span className="theme-option-copy">
              <span className="text-sm font-medium">{t(themeOption.label)}</span>
              {themeOption.value === theme && (
                <span className="text-xs font-normal text-neutral-500 dark:text-neutral-400">
                  {t("Active")}
                </span>
              )}
            </span>
          </Button>
        ))}
      </div>

      <div className="mt-4 rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
        <div className="flex items-center justify-between">
          <span className="text-sm text-neutral-600 dark:text-neutral-400">
            {t("Current theme")}
          </span>
          <span className="rounded bg-neutral-100 px-2 py-1 text-xs font-medium uppercase text-neutral-700 dark:bg-neutral-800 dark:text-neutral-300">
            {t(effectiveTheme === "dark" ? "Dark" : "Light")}
          </span>
        </div>
      </div>

      <div className="mt-4 rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
        <div className="flex items-center justify-between gap-4">
          <span className="text-sm text-neutral-600 dark:text-neutral-400">{t("Language")}</span>
          <Select
            value={locale}
            onValueChange={(value) => setLocale(value as LocalePreference)}
            ariaLabel={t("Language")}
            className="appearance-locale-select"
            options={[
              { value: "system", label: t("Use browser language") },
              { value: "en", label: t("English") },
              { value: "zh-CN", label: t("简体中文") },
            ]}
          />
        </div>
      </div>
    </section>
  );
}
