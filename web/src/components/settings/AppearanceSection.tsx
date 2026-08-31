import { Sun, Moon, Monitor, Globe } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useUIStore, type Theme } from "../../stores/useUIStore";
import type { LocalePreference } from "../../i18n";
import { Select } from "../ui";

const themes: { value: Theme; label: string; icon: React.ComponentType<{ className?: string }> }[] =
  [
    {
      value: "light",
      label: "Light",
      icon: Sun,
    },
    {
      value: "dark",
      label: "Dark",
      icon: Moon,
    },
    {
      value: "system",
      label: "System",
      icon: Monitor,
    },
  ];

export function AppearanceSection() {
  const { t } = useTranslation();
  const theme = useUIStore((s) => s.theme);
  const locale = useUIStore((s) => s.locale);
  const setTheme = useUIStore((s) => s.setTheme);
  const setLocale = useUIStore((s) => s.setLocale);

  return (
    <section className="space-y-6">
      <div>
        <h2 className="text-base font-semibold text-neutral-900 dark:text-neutral-100">
          {t("Appearance")}
        </h2>
        <p className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">
          {t("Choose your preferred color theme")}
        </p>
      </div>

      <div className="space-y-3">
        <label className="text-xs font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
          {t("Theme")}
        </label>
        <div className="grid grid-cols-3 gap-3">
          {themes.map((themeOption) => {
            const Icon = themeOption.icon;
            const isActive = themeOption.value === theme;
            return (
              <button
                type="button"
                key={themeOption.value}
                onClick={() => setTheme(themeOption.value)}
                className={`relative flex flex-col items-center justify-center gap-2.5 rounded-xl border p-4 text-center transition-all ${
                  isActive
                    ? "border-[var(--accent)] bg-[var(--selected)] ring-1 ring-[var(--accent)] shadow-xs dark:bg-neutral-800/90"
                    : "border-neutral-200/90 bg-white hover:border-neutral-300 hover:bg-neutral-50/70 dark:border-neutral-800 dark:bg-neutral-900/60 dark:hover:border-neutral-700 dark:hover:bg-neutral-800/40"
                }`}
              >
                <div
                  className={`flex h-10 w-10 items-center justify-center rounded-lg ${
                    isActive
                      ? "bg-white text-[var(--accent-strong)] shadow-xs dark:bg-neutral-700 dark:text-neutral-100"
                      : "bg-neutral-100 text-neutral-600 dark:bg-neutral-800 dark:text-neutral-400"
                  }`}
                >
                  <Icon className="h-5 w-5" />
                </div>
                <div>
                  <span
                    className={`block text-xs font-semibold ${
                      isActive
                        ? "text-neutral-900 dark:text-neutral-100"
                        : "text-neutral-700 dark:text-neutral-300"
                    }`}
                  >
                    {t(themeOption.label)}
                  </span>
                  {isActive ? (
                    <span className="mt-0.5 inline-block text-[10px] font-medium text-[var(--accent-strong)] dark:text-blue-400">
                      {t("Active")}
                    </span>
                  ) : (
                    <span className="mt-0.5 inline-block text-[10px] text-transparent select-none">
                      -
                    </span>
                  )}
                </div>
              </button>
            );
          })}
        </div>
      </div>

      <div className="space-y-3">
        <label className="text-xs font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
          {t("Language")}
        </label>
        <div className="flex items-center justify-between gap-4 rounded-xl border border-neutral-200/80 bg-neutral-50/50 p-3.5 dark:border-neutral-800/80 dark:bg-neutral-800/30">
          <div className="flex items-center gap-3 min-w-0">
            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-white shadow-xs text-neutral-600 dark:bg-neutral-800 dark:text-neutral-300">
              <Globe className="h-4 w-4" />
            </div>
            <div className="min-w-0">
              <span className="block truncate text-xs font-medium text-neutral-900 dark:text-neutral-100">
                {t("Language")}
              </span>
              <span className="block truncate text-[11px] text-neutral-500 dark:text-neutral-400">
                {t("Use browser language")} / English / 简体中文
              </span>
            </div>
          </div>
          <Select
            value={locale}
            onValueChange={(value) => setLocale(value as LocalePreference)}
            ariaLabel={t("Language")}
            className="appearance-locale-select shrink-0"
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
