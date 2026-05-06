import { X, Sun, Moon, Monitor, ArrowLeft } from "lucide-react";
import {
  useUIStore,
  getEffectiveTheme,
  type Theme,
} from "../../stores/useUIStore";

const themes: { value: Theme; label: string; icon: React.ReactNode }[] = [
  { value: "light", label: "Light", icon: <Sun className="h-8 w-8" /> },
  { value: "dark", label: "Dark", icon: <Moon className="h-8 w-8" /> },
  { value: "system", label: "System", icon: <Monitor className="h-8 w-8" /> },
];

export function SettingsView() {
  const theme = useUIStore((s) => s.theme);
  const setTheme = useUIStore((s) => s.setTheme);
  const navigateToChat = useUIStore((s) => s.navigateToChat);

  const effectiveTheme = getEffectiveTheme(theme);

  return (
    <div className="mx-auto flex h-full max-w-2xl flex-col overflow-y-auto p-6">
      {/* Header */}
      <div className="mb-6 flex items-center gap-3">
        <button
          onClick={() => navigateToChat()}
          className="rounded p-1.5 text-neutral-500 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
          aria-label="Back"
        >
          <ArrowLeft className="h-5 w-5" />
        </button>
        <h1 className="text-lg font-semibold text-neutral-900 dark:text-neutral-100">
          Settings
        </h1>
      </div>

      {/* Appearance */}
      <section className="mb-8">
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

      {/* Connection */}
      <section className="mb-8">
        <h2 className="mb-1 text-sm font-medium text-neutral-900 dark:text-neutral-100">
          Connection
        </h2>
        <p className="mb-3 text-sm text-neutral-500 dark:text-neutral-400">
          Server connection status
        </p>
        <div className="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
          <ConnectionStatus />
        </div>
      </section>

      {/* About */}
      <section>
        <h2 className="mb-1 text-sm font-medium text-neutral-900 dark:text-neutral-100">
          About
        </h2>
        <p className="mb-3 text-sm text-neutral-500 dark:text-neutral-400">
          TiDev Web Frontend
        </p>
        <div className="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
          <div className="flex items-center justify-between">
            <span className="text-sm text-neutral-600 dark:text-neutral-400">
              Version
            </span>
            <span className="text-sm text-neutral-900 dark:text-neutral-100">
              0.1.0
            </span>
          </div>
        </div>
      </section>
    </div>
  );
}

function ConnectionStatus() {
  const connectionStatus = useUIStore((s) => s.connectionStatus);

  const statusConfig: {
    connected: { color: string; label: string };
    disconnected: { color: string; label: string };
    connecting: { color: string; label: string };
  } = {
    connected: { color: "text-green-600", label: "Connected" },
    disconnected: { color: "text-red-600", label: "Disconnected" },
    connecting: { color: "text-yellow-600", label: "Connecting..." },
  };

  const config = statusConfig[connectionStatus];

  return (
    <div className="flex items-center justify-between">
      <span className="text-sm text-neutral-600 dark:text-neutral-400">
        Server
      </span>
      <span className={`flex items-center gap-1.5 text-sm font-medium ${config.color}`}>
        <span className={`h-2 w-2 rounded-full ${
          connectionStatus === "connected" ? "bg-green-500" :
          connectionStatus === "connecting" ? "bg-yellow-500" :
          "bg-red-500"
        }`} />
        {config.label}
      </span>
    </div>
  );
}
