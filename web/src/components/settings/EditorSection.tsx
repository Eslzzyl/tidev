import { Columns2, Rows, Type } from "lucide-react";
import { useUIStore } from "../../stores/useUIStore";
import { useTranslation } from "react-i18next";
import { Input, Slider } from "../ui";

const FONT_SIZES = [12, 13, 14, 15, 16, 18, 20];

export function EditorSection() {
  const { t } = useTranslation();
  const settings = useUIStore((s) => s.settings);
  const updateSettings = useUIStore((s) => s.updateSettings);

  return (
    <section className="space-y-6">
      <div>
        <h2 className="text-base font-semibold text-neutral-900 dark:text-neutral-100">
          {t("Editor")}
        </h2>
        <p className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">
          {t("Customize the display fonts and code diff layout")}
        </p>
      </div>

      {/* Typography Card */}
      <div className="space-y-3">
        <label className="text-xs font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
          {t("Typography")}
        </label>
        <div className="rounded-xl border border-neutral-200/80 bg-neutral-50/50 p-4 divide-y divide-neutral-200/60 dark:border-neutral-800/80 dark:bg-neutral-800/30 dark:divide-neutral-800/60">
          {/* UI Font */}
          <div className="pb-3.5">
            <div className="flex items-center justify-between mb-1.5">
              <label className="text-xs font-medium text-neutral-900 dark:text-neutral-100">
                {t("UI Font")}
              </label>
              <span className="text-[11px] text-neutral-400 dark:text-neutral-500">
                {t("Font family for the user interface")}
              </span>
            </div>
            <Input
              type="text"
              value={settings.fontFamily}
              onChange={(e) => updateSettings({ fontFamily: e.target.value })}
              placeholder="Inter, system-ui, sans-serif"
            />
          </div>

          {/* Monospace Font */}
          <div className="pt-3.5">
            <div className="flex items-center justify-between mb-1.5">
              <label className="text-xs font-medium text-neutral-900 dark:text-neutral-100">
                {t("Monospace Font")}
              </label>
              <span className="text-[11px] text-neutral-400 dark:text-neutral-500">
                {t("Font family for code blocks and diffs")}
              </span>
            </div>
            <Input
              type="text"
              value={settings.monoFontFamily}
              onChange={(e) => updateSettings({ monoFontFamily: e.target.value })}
              placeholder="JetBrains Mono, Fira Code, monospace"
              className="font-mono"
            />
          </div>
        </div>
      </div>

      {/* Font Size Card */}
      <div className="space-y-3">
        <label className="text-xs font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
          {t("Font Size")}
        </label>
        <div className="rounded-xl border border-neutral-200/80 bg-neutral-50/50 p-4 dark:border-neutral-800/80 dark:bg-neutral-800/30">
          <div className="flex items-center justify-between gap-4 mb-3">
            <div className="flex items-center gap-2">
              <Type className="h-4 w-4 text-neutral-500" />
              <span className="text-xs font-medium text-neutral-900 dark:text-neutral-100">
                {t("Base font size")}
              </span>
            </div>
            <span className="rounded-md bg-white px-2 py-0.5 text-xs font-semibold tabular-nums text-neutral-800 shadow-xs dark:bg-neutral-800 dark:text-neutral-200">
              {settings.fontSize}px
            </span>
          </div>

          <div className="flex items-center gap-4">
            <Slider
              min={12}
              max={20}
              step={1}
              value={[settings.fontSize]}
              onValueChange={([value]) => updateSettings({ fontSize: value })}
              className="flex-1"
            />
          </div>

          <div className="mt-3 flex flex-wrap gap-1.5">
            {FONT_SIZES.map((s) => (
              <button
                type="button"
                key={s}
                onClick={() => updateSettings({ fontSize: s })}
                className={`rounded-md px-2.5 py-1 text-xs font-medium transition-all ${
                  settings.fontSize === s
                    ? "bg-[var(--accent)] text-white shadow-xs"
                    : "bg-white text-neutral-600 hover:bg-neutral-100 dark:bg-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-700"
                }`}
              >
                {s}px
              </button>
            ))}
          </div>
        </div>
      </div>

      {/* Diff Layout */}
      <div className="space-y-3">
        <label className="text-xs font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
          {t("Diff Layout")}
        </label>
        <div className="grid grid-cols-2 gap-3">
          <button
            type="button"
            onClick={() => updateSettings({ diffLayout: "side-by-side" })}
            className={`flex items-start gap-3 rounded-xl border p-3.5 text-left transition-all ${
              settings.diffLayout === "side-by-side"
                ? "border-[var(--accent)] bg-[var(--selected)] ring-1 ring-[var(--accent)] dark:bg-neutral-800/90"
                : "border-neutral-200/90 bg-white hover:border-neutral-300 hover:bg-neutral-50/70 dark:border-neutral-800 dark:bg-neutral-900/60 dark:hover:border-neutral-700 dark:hover:bg-neutral-800/40"
            }`}
          >
            <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-neutral-100 text-neutral-700 dark:bg-neutral-800 dark:text-neutral-300">
              <Columns2 className="h-4 w-4" />
            </div>
            <div className="min-w-0">
              <span className="block text-xs font-semibold text-neutral-900 dark:text-neutral-100">
                {t("Side by Side")}
              </span>
              <span className="block text-[11px] text-neutral-500 dark:text-neutral-400">
                {t("Old | New")}
              </span>
            </div>
          </button>

          <button
            type="button"
            onClick={() => updateSettings({ diffLayout: "inline" })}
            className={`flex items-start gap-3 rounded-xl border p-3.5 text-left transition-all ${
              settings.diffLayout === "inline"
                ? "border-[var(--accent)] bg-[var(--selected)] ring-1 ring-[var(--accent)] dark:bg-neutral-800/90"
                : "border-neutral-200/90 bg-white hover:border-neutral-300 hover:bg-neutral-50/70 dark:border-neutral-800 dark:bg-neutral-900/60 dark:hover:border-neutral-700 dark:hover:bg-neutral-800/40"
            }`}
          >
            <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-neutral-100 text-neutral-700 dark:bg-neutral-800 dark:text-neutral-300">
              <Rows className="h-4 w-4" />
            </div>
            <div className="min-w-0">
              <span className="block text-xs font-semibold text-neutral-900 dark:text-neutral-100">
                {t("Inline")}
              </span>
              <span className="block text-[11px] text-neutral-500 dark:text-neutral-400">
                {t("Unified view")}
              </span>
            </div>
          </button>
        </div>
      </div>
    </section>
  );
}
