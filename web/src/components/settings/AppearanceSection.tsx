import { Sun, Moon, Monitor } from "lucide-react";
import {
  useUIStore,
  getEffectiveTheme,
  type Theme,
} from "../../stores/useUIStore";

const themes: { value: Theme; label: string; icon: React.ReactNode }[] = [
  { value: "light", label: "Light", icon: <Sun className="h-8 w-8 text-neutral-900 dark:text-neutral-100" /> },
  { value: "dark", label: "Dark", icon: <Moon className="h-8 w-8 text-neutral-900 dark:text-neutral-100" /> },
  { value: "system", label: "System", icon: <Monitor className="h-8 w-8 text-neutral-900 dark:text-neutral-100" /> },
];

export function AppearanceSection() {
  const theme = useUIStore((s) => s.theme);
  const setTheme = useUIStore((s) => s.setTheme);
  const effectiveTheme = getEffectiveTheme(theme);

  return (
    <section>
      <h2 className="mb-1 text-sm font-medium text-neutral-900 dark:text-neutral-100">
        Appearance
      </h2>
      <p className="mb-4 text-sm text-neutral-500 dark:text-neutral-400">
        Choose your preferred color theme
      </p>

      <div className="grid grid-cols-3 gap-3">
        {themes.map((t) => (
          <button
            key={t.value}
            onClick={() => setTheme(t.value)}
            className={`flex flex-col items-center gap-2 rounded-lg border p-4 transition-all ${
              t.value === theme
                ? "border-neutral-900 bg-neutral-50 dark:border-neutral-100 dark:bg-neutral-800"
                : "border-neutral-200 hover:border-neutral-300 dark:border-neutral-700 dark:hover:border-neutral-600"
            }`}
          >
            {t.icon}
            <span className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
              {t.label}
            </span>
            {t.value === theme && (
              <span className="text-xs text-neutral-500 dark:text-neutral-400">
                Active
              </span>
            )}
          </button>
        ))}
      </div>

      <div className="mt-4 rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
        <div className="flex items-center justify-between">
          <span className="text-sm text-neutral-600 dark:text-neutral-400">
            Current theme
          </span>
          <span className="rounded bg-neutral-100 px-2 py-1 text-xs font-medium uppercase text-neutral-700 dark:bg-neutral-800 dark:text-neutral-300">
            {effectiveTheme}
          </span>
        </div>
      </div>
    </section>
  );
}
