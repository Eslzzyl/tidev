import { X, Sun, Moon, Monitor, ArrowLeft, Type, Columns, Layout as LayoutIcon, Keyboard } from "lucide-react";
import {
  useUIStore,
  getEffectiveTheme,
  type Theme,
  type SettingsState,
} from "../../stores/useUIStore";

const themes: { value: Theme; label: string; icon: React.ReactNode }[] = [
  { value: "light", label: "Light", icon: <Sun className="h-8 w-8" /> },
  { value: "dark", label: "Dark", icon: <Moon className="h-8 w-8" /> },
  { value: "system", label: "System", icon: <Monitor className="h-8 w-8" /> },
];

const FONT_SIZES = [12, 13, 14, 15, 16, 18, 20];

export function SettingsView() {
  const theme = useUIStore((s) => s.theme);
  const setTheme = useUIStore((s) => s.setTheme);
  const settings = useUIStore((s) => s.settings);
  const updateSettings = useUIStore((s) => s.updateSettings);
  const navigateToChat = useUIStore((s) => s.navigateToChat);

  const effectiveTheme = getEffectiveTheme(theme);

  return (
    <div className="mx-auto flex h-full w-full max-w-2xl flex-col overflow-y-auto p-4 sm:p-6 lg:max-w-3xl xl:max-w-4xl">
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
        <h2 className="mb-1 flex items-center gap-2 text-sm font-medium text-neutral-900 dark:text-neutral-100">
          <Sun className="h-4 w-4" /> Appearance
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

      {/* Font */}
      <section className="mb-8">
        <h2 className="mb-1 flex items-center gap-2 text-sm font-medium text-neutral-900 dark:text-neutral-100">
          <Type className="h-4 w-4" /> Font
        </h2>
        <p className="mb-4 text-sm text-neutral-500 dark:text-neutral-400">
          Customize the display fonts
        </p>

        <div className="space-y-4">
          <div className="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
            <label className="mb-1.5 block text-sm font-medium text-neutral-700 dark:text-neutral-300">
              UI Font
            </label>
            <input
              type="text"
              value={settings.fontFamily}
              onChange={(e) => updateSettings({ fontFamily: e.target.value })}
              className="w-full rounded border border-neutral-300 bg-white px-3 py-1.5 text-sm text-neutral-900 dark:border-neutral-600 dark:bg-neutral-900 dark:text-neutral-100"
              placeholder="Inter, system-ui, sans-serif"
            />
            <p className="mt-1 text-xs text-neutral-500">
              Font family for the user interface
            </p>
          </div>

          <div className="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
            <label className="mb-1.5 block text-sm font-medium text-neutral-700 dark:text-neutral-300">
              Monospace Font
            </label>
            <input
              type="text"
              value={settings.monoFontFamily}
              onChange={(e) => updateSettings({ monoFontFamily: e.target.value })}
              className="w-full rounded border border-neutral-300 bg-white px-3 py-1.5 text-sm text-neutral-900 dark:border-neutral-600 dark:bg-neutral-900 dark:text-neutral-100"
              placeholder="JetBrains Mono, Fira Code, monospace"
            />
            <p className="mt-1 text-xs text-neutral-500">
              Font family for code blocks and diffs
            </p>
          </div>

          <div className="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
            <label className="mb-1.5 block text-sm font-medium text-neutral-700 dark:text-neutral-300">
              Font Size
            </label>
            <div className="flex items-center gap-3">
              <input
                type="range"
                min={12}
                max={20}
                step={1}
                value={settings.fontSize}
                onChange={(e) => updateSettings({ fontSize: parseInt(e.target.value) })}
                className="flex-1 accent-neutral-900 dark:accent-neutral-100"
              />
              <span className="min-w-[2rem] text-right text-sm tabular-nums text-neutral-700 dark:text-neutral-300">
                {settings.fontSize}px
              </span>
            </div>
            <div className="mt-1 flex justify-between text-xs text-neutral-500">
              {FONT_SIZES.map((s) => (
                <button
                  key={s}
                  onClick={() => updateSettings({ fontSize: s })}
                  className={`rounded px-1.5 py-0.5 transition-colors ${
                    settings.fontSize === s
                      ? "bg-neutral-200 font-medium text-neutral-900 dark:bg-neutral-700 dark:text-neutral-100"
                      : "hover:text-neutral-700 dark:hover:text-neutral-300"
                  }`}
                >
                  {s}
                </button>
              ))}
            </div>
          </div>
        </div>
      </section>

      {/* Diff Layout */}
      <section className="mb-8">
        <h2 className="mb-1 flex items-center gap-2 text-sm font-medium text-neutral-900 dark:text-neutral-100">
          <Columns className="h-4 w-4" /> Diff Layout
        </h2>
        <p className="mb-4 text-sm text-neutral-500 dark:text-neutral-400">
          Default view mode for code diffs
        </p>

        <div className="grid grid-cols-2 gap-3">
          <button
            onClick={() => updateSettings({ diffLayout: "side-by-side" })}
            className={`flex flex-col items-center gap-2 rounded-lg border p-4 transition-all ${
              settings.diffLayout === "side-by-side"
                ? "border-neutral-900 bg-neutral-50 dark:border-neutral-100 dark:bg-neutral-800"
                : "border-neutral-200 hover:border-neutral-300 dark:border-neutral-700 dark:hover:border-neutral-600"
            }`}
          >
            <Columns className="h-6 w-6" />
            <span className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
              Side by Side
            </span>
            <span className="text-xs text-neutral-500">
              Old | New
            </span>
          </button>
          <button
            onClick={() => updateSettings({ diffLayout: "inline" })}
            className={`flex flex-col items-center gap-2 rounded-lg border p-4 transition-all ${
              settings.diffLayout === "inline"
                ? "border-neutral-900 bg-neutral-50 dark:border-neutral-100 dark:bg-neutral-800"
                : "border-neutral-200 hover:border-neutral-300 dark:border-neutral-700 dark:hover:border-neutral-600"
            }`}
          >
            <LayoutIcon className="h-6 w-6" />
            <span className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
              Inline
            </span>
            <span className="text-xs text-neutral-500">
              Unified view
            </span>
          </button>
        </div>
      </section>

      {/* Behavior */}
      <section className="mb-8">
        <h2 className="mb-1 flex items-center gap-2 text-sm font-medium text-neutral-900 dark:text-neutral-100">
          <Keyboard className="h-4 w-4" /> Behavior
        </h2>
        <p className="mb-4 text-sm text-neutral-500 dark:text-neutral-400">
          Customize how the chat input behaves
        </p>

        <div className="space-y-3">
          <label className="flex items-center justify-between rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
            <div>
              <span className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
                Enter to send
              </span>
              <p className="text-xs text-neutral-500">
                Press Enter to send, Shift+Enter for new line
              </p>
            </div>
            <button
              role="switch"
              aria-checked={settings.enterToSend}
              onClick={() => updateSettings({ enterToSend: !settings.enterToSend })}
              className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors ${
                settings.enterToSend
                  ? "bg-neutral-900 dark:bg-neutral-100"
                  : "bg-neutral-300 dark:bg-neutral-600"
              }`}
            >
              <span
                className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                  settings.enterToSend ? "translate-x-4.5" : "translate-x-1"
                }`}
              />
            </button>
          </label>
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
