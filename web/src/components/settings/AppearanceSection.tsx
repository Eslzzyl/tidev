import { Sun, Moon, Monitor } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useUIStore, getEffectiveTheme, type Theme } from "../../stores/useUIStore";
import type { LocalePreference } from "../../i18n";

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
          <button
            key={themeOption.value}
            onClick={() => setTheme(themeOption.value)}
            className={`flex flex-col items-center gap-2 rounded-lg border p-4 transition-all ${
              themeOption.value === theme
                ? "border-neutral-900 bg-neutral-50 dark:border-neutral-100 dark:bg-neutral-800"
                : "border-neutral-200 hover:border-neutral-300 dark:border-neutral-700 dark:hover:border-neutral-600"
            }`}
          >
            {themeOption.icon}
            <span className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
              {t(themeOption.label)}
            </span>
            {themeOption.value === theme && (
              <span className="text-xs text-neutral-500 dark:text-neutral-400">{t("Active")}</span>
            )}
          </button>
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
        <label className="flex items-center justify-between gap-4">
          <span className="text-sm text-neutral-600 dark:text-neutral-400">{t("Language")}</span>
          <select
            value={locale}
            onChange={(event) => setLocale(event.target.value as LocalePreference)}
            className="rounded border border-neutral-300 bg-white px-2 py-1 text-sm text-neutral-700 outline-none dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-200"
          >
            <option value="system">{t("Use browser language")}</option>
            <option value="en">{t("English")}</option>
            <option value="zh-CN">{t("简体中文")}</option>
          </select>
        </label>
      </div>
    </section>
  );
}
