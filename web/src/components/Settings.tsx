import { useUIStore, getEffectiveTheme, type Theme } from '../stores/useUIStore';
import { useClickOutside } from '../hooks/useClickOutside';

const themes: { value: Theme; label: string; icon: string }[] = [
  { value: 'light', label: 'Light', icon: '☀️' },
  { value: 'dark', label: 'Dark', icon: '🌙' },
  { value: 'system', label: 'System', icon: '💻' },
];

export function Settings() {
  const settingsOpen = useUIStore((s) => s.settingsOpen);
  const theme = useUIStore((s) => s.theme);
  const setTheme = useUIStore((s) => s.setTheme);
  const closeSettings = useUIStore((s) => s.closeSettings);

  const modalRef = useClickOutside(closeSettings);

  if (!settingsOpen) return null;

  const effectiveTheme = getEffectiveTheme(theme);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      <div
        ref={modalRef}
        className="w-full max-w-md rounded-xl bg-white shadow-2xl dark:bg-neutral-900"
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-neutral-200 px-6 py-4 dark:border-neutral-800">
          <h2 className="text-lg font-semibold text-neutral-900 dark:text-neutral-100">Settings</h2>
          <button
            onClick={closeSettings}
            className="rounded p-1 text-neutral-500 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
            aria-label="Close settings"
          >
            <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Content */}
        <div className="p-6">
          <div>
            <h3 className="mb-3 text-sm font-medium text-neutral-900 dark:text-neutral-100">Appearance</h3>
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
                      ? 'border-neutral-900 bg-neutral-50 dark:border-neutral-100 dark:bg-neutral-800'
                      : 'border-neutral-200 hover:border-neutral-300 dark:border-neutral-700 dark:hover:border-neutral-600'
                  }`}
                >
                  <span className="text-2xl">{t.icon}</span>
                  <span className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
                    {t.label}
                  </span>
                  {t.value === theme && (
                    <span className="text-xs text-neutral-500 dark:text-neutral-400">Active</span>
                  )}
                </button>
              ))}
            </div>
          </div>

          <div className="mt-6 rounded-lg border border-neutral-200 p-4 dark:border-neutral-800">
            <div className="flex items-center justify-between">
              <span className="text-sm text-neutral-600 dark:text-neutral-400">Current theme</span>
              <span className="rounded bg-neutral-100 px-2 py-1 text-xs font-medium uppercase text-neutral-700 dark:bg-neutral-800 dark:text-neutral-300">
                {effectiveTheme}
              </span>
            </div>
          </div>
        </div>

        {/* Footer */}
        <div className="border-t border-neutral-200 px-6 py-4 dark:border-neutral-800">
          <p className="text-center text-xs text-neutral-500 dark:text-neutral-400">
            Settings are saved automatically
          </p>
        </div>
      </div>
    </div>
  );
}
