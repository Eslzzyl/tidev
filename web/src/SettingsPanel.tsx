import { useCallback, useEffect, useState } from "react";
import {
  Info,
  Keyboard,
  Lock,
  Moon,
  Palette,
  Sun,
  Terminal as TerminalIcon,
  Type,
  X,
  Monitor,
} from "lucide-react";

import { api, getAuthToken, setAuthToken } from "./api";

type Theme = "light" | "dark" | "system";
type Category = "appearance" | "editor" | "interaction" | "terminal" | "security" | "about";

interface Settings {
  theme: Theme;
  fontFamily: string;
  monoFontFamily: string;
  fontSize: number;
  diffLayout: "inline" | "side-by-side";
  enterToSend: boolean;
  terminalShell: string;
}

const defaults: Settings = {
  theme: "system",
  fontFamily: "Inter, system-ui, sans-serif",
  monoFontFamily: "JetBrains Mono, Fira Code, monospace",
  fontSize: 14,
  diffLayout: "side-by-side",
  enterToSend: true,
  terminalShell: "",
};

const categories: { id: Category; label: string; icon: typeof Palette }[] = [
  { id: "appearance", label: "Appearance", icon: Palette },
  { id: "editor", label: "Editor", icon: Type },
  { id: "interaction", label: "Interaction", icon: Keyboard },
  { id: "terminal", label: "Terminal", icon: TerminalIcon },
  { id: "security", label: "Security", icon: Lock },
  { id: "about", label: "About", icon: Info },
];

function loadSettings(): Settings {
  try {
    const value = JSON.parse(
      localStorage.getItem("tidev-ui-settings") ?? "null",
    ) as Partial<Settings> | null;
    return value ? { ...defaults, ...value } : defaults;
  } catch {
    return defaults;
  }
}

function setThemeAttribute(theme: Theme) {
  if (theme === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = theme;
}

export default function SettingsPanel({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [category, setCategory] = useState<Category>("appearance");
  const [settings, setSettings] = useState<Settings>(loadSettings);

  useEffect(() => {
    if (!open) return;
    setSettings(loadSettings());
  }, [open]);

  useEffect(() => {
    localStorage.setItem("tidev-ui-settings", JSON.stringify(settings));
    window.dispatchEvent(new CustomEvent("tidev-ui-settings-changed", { detail: settings }));
    setThemeAttribute(settings.theme);
    document.documentElement.style.setProperty("--ui-font-size", `${settings.fontSize}px`);
    document.documentElement.style.setProperty("--ui-font-family", settings.fontFamily);
    document.documentElement.style.setProperty("--ui-mono-font", settings.monoFontFamily);
  }, [settings]);

  if (!open) return null;

  const update = useCallback(
    (patch: Partial<Settings>) => setSettings((current) => ({ ...current, ...patch })),
    [],
  );

  return (
    <div
      className="settings-overlay"
      role="dialog"
      aria-modal="true"
      aria-label="Settings"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="settings-panel">
        <div className="settings-header">
          <h2>Settings</h2>
          <button className="settings-close" onClick={onClose} aria-label="Close settings">
            <X size={18} />
          </button>
        </div>
        <div className="settings-body">
          <nav className="settings-nav">
            {categories.map(({ id, label, icon: Icon }) => (
              <button
                key={id}
                className={category === id ? "settings-nav-item active" : "settings-nav-item"}
                onClick={() => setCategory(id)}
              >
                <Icon size={16} />
                <span>{label}</span>
              </button>
            ))}
          </nav>
          <div className="settings-content">
            {category === "appearance" && <AppearanceSection settings={settings} update={update} />}
            {category === "editor" && <EditorSection settings={settings} update={update} />}
            {category === "interaction" && (
              <InteractionSection settings={settings} update={update} />
            )}
            {category === "terminal" && <TerminalSection settings={settings} update={update} />}
            {category === "security" && <SecuritySection />}
            {category === "about" && <AboutSection />}
          </div>
        </div>
        <div className="settings-footer">Settings are saved automatically</div>
      </div>
    </div>
  );
}

function SectionIntro({ title, text }: { title: string; text: string }) {
  return (
    <>
      <h3 className="settings-title">{title}</h3>
      <p className="settings-description">{text}</p>
    </>
  );
}

function AppearanceSection({
  settings,
  update,
}: {
  settings: Settings;
  update: (patch: Partial<Settings>) => void;
}) {
  const options: { value: Theme; label: string; icon: typeof Sun }[] = [
    { value: "light", label: "Light", icon: Sun },
    { value: "dark", label: "Dark", icon: Moon },
    { value: "system", label: "System", icon: Monitor },
  ];
  return (
    <section>
      <SectionIntro title="Appearance" text="Choose your preferred color theme" />
      <div className="theme-options">
        {options.map(({ value, label, icon: Icon }) => (
          <button
            key={value}
            className={settings.theme === value ? "theme-option active" : "theme-option"}
            onClick={() => update({ theme: value })}
          >
            <Icon size={27} />
            <span>{label}</span>
            {settings.theme === value ? <small>Active</small> : null}
          </button>
        ))}
      </div>
      <div className="settings-card settings-row">
        <span>Current theme</span>
        <strong>{settings.theme}</strong>
      </div>
    </section>
  );
}

function EditorSection({
  settings,
  update,
}: {
  settings: Settings;
  update: (patch: Partial<Settings>) => void;
}) {
  return (
    <section>
      <SectionIntro title="Editor" text="Customize display fonts and code diff layout" />
      <div className="settings-stack">
        <label className="settings-card settings-field">
          <span>UI Font</span>
          <input
            value={settings.fontFamily}
            onChange={(event) => update({ fontFamily: event.target.value })}
            placeholder="Inter, system-ui, sans-serif"
          />
          <small>Font family for the user interface</small>
        </label>
        <label className="settings-card settings-field">
          <span>Monospace Font</span>
          <input
            value={settings.monoFontFamily}
            onChange={(event) => update({ monoFontFamily: event.target.value })}
            placeholder="JetBrains Mono, Fira Code, monospace"
          />
          <small>Font family for code blocks and diffs</small>
        </label>
        <div className="settings-card settings-field">
          <span>Font Size</span>
          <div className="range-row">
            <input
              type="range"
              min="12"
              max="20"
              value={settings.fontSize}
              onChange={(event) => update({ fontSize: Number(event.target.value) })}
            />
            <strong>{settings.fontSize}px</strong>
          </div>
        </div>
        <div className="settings-card settings-field">
          <span>Diff Layout</span>
          <div className="choice-grid">
            <button
              className={settings.diffLayout === "side-by-side" ? "choice active" : "choice"}
              onClick={() => update({ diffLayout: "side-by-side" })}
            >
              Side by Side<small>Old | New</small>
            </button>
            <button
              className={settings.diffLayout === "inline" ? "choice active" : "choice"}
              onClick={() => update({ diffLayout: "inline" })}
            >
              Inline<small>Unified view</small>
            </button>
          </div>
        </div>
      </div>
    </section>
  );
}

function InteractionSection({
  settings,
  update,
}: {
  settings: Settings;
  update: (patch: Partial<Settings>) => void;
}) {
  return (
    <section>
      <SectionIntro title="Interaction" text="Customize how the chat input behaves" />
      <div className="settings-card settings-row">
        <div>
          <strong>Enter to send</strong>
          <small>Press Enter to send, Shift+Enter for a new line</small>
        </div>
        <button
          className={settings.enterToSend ? "toggle on" : "toggle"}
          role="switch"
          aria-checked={settings.enterToSend}
          onClick={() => update({ enterToSend: !settings.enterToSend })}
        >
          <span />
        </button>
      </div>
    </section>
  );
}

function TerminalSection({
  settings,
  update,
}: {
  settings: Settings;
  update: (patch: Partial<Settings>) => void;
}) {
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    void api
      .terminalShell()
      .then(({ shell, configured }) => {
        if (!disposed && configured) update({ terminalShell: shell });
      })
      .catch((reason) => {
        if (!disposed) setError(reason instanceof Error ? reason.message : "Failed to load shell");
      })
      .finally(() => {
        if (!disposed) setLoading(false);
      });
    return () => {
      disposed = true;
    };
  }, [update]);

  const persist = (shell: string) => {
    update({ terminalShell: shell });
    setError(null);
    void api.setTerminalShell(shell).catch((reason) => {
      setError(reason instanceof Error ? reason.message : "Failed to save shell");
    });
  };

  return (
    <section>
      <SectionIntro title="Terminal" text="Choose which shell to use in the terminal" />
      <div className="settings-card settings-field">
        <span>Shell</span>
        <select
          value={settings.terminalShell || "__default__"}
          disabled={loading}
          onChange={(event) => {
            const value = event.target.value;
            if (value === "__default__") persist("");
            else if (value === "__custom__") persist(settings.terminalShell);
            else persist(value);
          }}
        >
          <option value="__default__">System default</option>
          <option value="/bin/bash">Bash (/bin/bash)</option>
          <option value="/bin/zsh">Zsh (/bin/zsh)</option>
          <option value="__custom__">Custom...</option>
        </select>
        {settings.terminalShell && !["/bin/bash", "/bin/zsh"].includes(settings.terminalShell) ? (
          <input
            value={settings.terminalShell}
            onChange={(event) => persist(event.target.value)}
            placeholder="/usr/local/bin/nushell"
          />
        ) : null}
        <small>
          {error ?? "Uses the server's shell environment variable when left at the default."}
        </small>
      </div>
    </section>
  );
}

function SecuritySection() {
  const [authRequired, setAuthRequired] = useState(false);
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void api.authStatus().then(({ auth_required }) => setAuthRequired(auth_required));
  }, []);

  const savePassword = async (remove = false) => {
    if (submitting) return;
    if (!remove && (!newPassword.trim() || newPassword !== confirmPassword)) return;
    setSubmitting(true);
    setSaved(false);
    setError(null);
    try {
      if (authRequired) {
        const current = currentPassword.trim();
        if (!current || !(await api.verifyAuthToken(current)).valid) {
          throw new Error("Current password is incorrect");
        }
        setAuthToken(current);
      }
      const next = remove ? "" : newPassword.trim();
      await api.configureAuthToken(next);
      setAuthToken(next);
      setAuthRequired(Boolean(next));
      setCurrentPassword("");
      setNewPassword("");
      setConfirmPassword("");
      setSaved(true);
    } catch (reason) {
      setSaved(false);
      setCurrentPassword("");
      setNewPassword("");
      setConfirmPassword("");
      setError(reason instanceof Error ? reason.message : "Failed to update password");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <section>
      <SectionIntro title="Security" text="Set a password to protect the web interface" />
      <div className="settings-card settings-field settings-stack">
        {authRequired ? (
          <label>
            <span>Current password</span>
            <input
              type="password"
              value={currentPassword}
              onChange={(event) => setCurrentPassword(event.target.value)}
              placeholder="Enter current password"
            />
          </label>
        ) : null}
        <label>
          <span>{authRequired ? "New password" : "Password"}</span>
          <input
            type="password"
            value={newPassword}
            onChange={(event) => setNewPassword(event.target.value)}
            placeholder="Enter password"
          />
        </label>
        <label>
          <span>Confirm password</span>
          <input
            type="password"
            value={confirmPassword}
            onChange={(event) => setConfirmPassword(event.target.value)}
            placeholder="Enter password again"
          />
        </label>
        <div className="settings-actions">
          <button
            className="settings-primary"
            disabled={
              submitting ||
              !newPassword.trim() ||
              newPassword !== confirmPassword ||
              (authRequired && !currentPassword.trim())
            }
            onClick={() => void savePassword()}
          >
            {submitting
              ? "Saving…"
              : saved
                ? "Saved"
                : authRequired
                  ? "Change password"
                  : "Set password"}
          </button>
          {authRequired ? (
            <button
              className="settings-danger"
              disabled={submitting || !currentPassword.trim()}
              onClick={() => void savePassword(true)}
            >
              Remove password
            </button>
          ) : null}
        </div>
        <small>
          {getAuthToken()
            ? "The current browser keeps the access token locally."
            : "A password protects every API and event-stream request."}
        </small>
        {error ? <small className="settings-error">{error}</small> : null}
      </div>
    </section>
  );
}

function AboutSection() {
  const [connected, setConnected] = useState<boolean | null>(null);

  useEffect(() => {
    let disposed = false;
    void api
      .health()
      .then(() => {
        if (!disposed) setConnected(true);
      })
      .catch(() => {
        if (!disposed) setConnected(false);
      });
    return () => {
      disposed = true;
    };
  }, []);

  return (
    <section>
      <SectionIntro title="About" text="tidev Web Frontend" />
      <div className="settings-stack">
        <div className="settings-card settings-row">
          <span>Version</span>
          <strong>0.9.0</strong>
        </div>
        <div className="settings-card settings-row">
          <span>Server</span>
          <strong className={connected ? "online-status" : "offline-status"}>
            <i />
            {connected === null ? "Checking…" : connected ? "Connected" : "Disconnected"}
          </strong>
        </div>
      </div>
    </section>
  );
}
