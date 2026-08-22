export const supportedLocales = ["en", "zh-CN"] as const;

export type AppLocale = (typeof supportedLocales)[number];
export type LocalePreference = AppLocale | "system";

export function resolveLocale(preference: LocalePreference): AppLocale {
  if (preference !== "system") return preference;

  if (typeof navigator !== "undefined" && navigator.language.toLowerCase().startsWith("zh")) {
    return "zh-CN";
  }

  return "en";
}

export function readPersistedLocale(): LocalePreference {
  if (typeof localStorage === "undefined") return "system";

  try {
    const stored = JSON.parse(localStorage.getItem("tidev-ui") ?? "null") as {
      state?: { locale?: LocalePreference };
    } | null;
    const locale = stored?.state?.locale;
    if (locale === "en" || locale === "zh-CN" || locale === "system") return locale;
  } catch {
    // Ignore malformed browser preferences and use the system locale.
  }

  return "system";
}
