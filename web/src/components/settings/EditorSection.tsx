import { useUIStore } from "../../stores/useUIStore";
import { useTranslation } from "react-i18next";
import { Button, Input, Slider } from "../ui";

const FONT_SIZES = [12, 13, 14, 15, 16, 18, 20];

export function EditorSection() {
  const { t } = useTranslation();
  const settings = useUIStore((s) => s.settings);
  const updateSettings = useUIStore((s) => s.updateSettings);

  return (
    <section>
      <h2 className="mb-1 text-base font-medium text-neutral-900 dark:text-neutral-100">
        {t("Editor")}
      </h2>
      <p className="mb-4 text-base text-neutral-500 dark:text-neutral-400">
        {t("Customize the display fonts and code diff layout")}
      </p>

      <div className="space-y-4">
        {/* UI Font */}
        <div className="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
          <label className="mb-1.5 block text-base font-medium text-neutral-700 dark:text-neutral-300">
            {t("UI Font")}
          </label>
          <Input
            type="text"
            value={settings.fontFamily}
            onChange={(e) => updateSettings({ fontFamily: e.target.value })}
            className="w-full rounded border border-neutral-300 bg-white px-3 py-1.5 text-base text-neutral-900 dark:border-neutral-600 dark:bg-neutral-900 dark:text-neutral-100"
            placeholder="Inter, system-ui, sans-serif"
          />
          <p className="mt-1 text-xs text-neutral-500">{t("Font family for the user interface")}</p>
        </div>

        {/* Monospace Font */}
        <div className="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
          <label className="mb-1.5 block text-base font-medium text-neutral-700 dark:text-neutral-300">
            {t("Monospace Font")}
          </label>
          <Input
            type="text"
            value={settings.monoFontFamily}
            onChange={(e) => updateSettings({ monoFontFamily: e.target.value })}
            className="w-full rounded border border-neutral-300 bg-white px-3 py-1.5 text-base text-neutral-900 dark:border-neutral-600 dark:bg-neutral-900 dark:text-neutral-100"
            placeholder="JetBrains Mono, Fira Code, monospace"
          />
          <p className="mt-1 text-xs text-neutral-500">
            {t("Font family for code blocks and diffs")}
          </p>
        </div>

        {/* Font Size */}
        <div className="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
          <label className="mb-1.5 block text-base font-medium text-neutral-700 dark:text-neutral-300">
            {t("Font Size")}
          </label>
          <div className="flex items-center gap-3">
            <Slider
              min={12}
              max={20}
              step={1}
              value={[settings.fontSize]}
              onValueChange={([value]) => updateSettings({ fontSize: value })}
              className="flex-1"
            />
            <span className="min-w-[2rem] text-right text-base tabular-nums text-neutral-700 dark:text-neutral-300">
              {settings.fontSize}px
            </span>
          </div>
          <div className="mt-1 flex justify-between text-xs text-neutral-500">
            {FONT_SIZES.map((s) => (
              <Button
                type="button"
                key={s}
                onClick={() => updateSettings({ fontSize: s })}
                variant={settings.fontSize === s ? "secondary" : "ghost"}
                size="sm"
              >
                {s}
              </Button>
            ))}
          </div>
        </div>

        {/* Diff Layout */}
        <div className="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
          <label className="mb-1.5 block text-base font-medium text-neutral-700 dark:text-neutral-300">
            {t("Diff Layout")}
          </label>
          <div className="grid grid-cols-2 gap-2">
            <Button
              type="button"
              onClick={() => updateSettings({ diffLayout: "side-by-side" })}
              className="editor-layout-option"
              variant="secondary"
              size="md"
              data-active={settings.diffLayout === "side-by-side" ? "true" : undefined}
            >
              <span className="text-base font-medium text-neutral-900 dark:text-neutral-100">
                {t("Side by Side")}
              </span>
              <span className="text-xs text-neutral-500">{t("Old | New")}</span>
            </Button>
            <Button
              type="button"
              onClick={() => updateSettings({ diffLayout: "inline" })}
              className="editor-layout-option"
              variant="secondary"
              size="md"
              data-active={settings.diffLayout === "inline" ? "true" : undefined}
            >
              <span className="text-base font-medium text-neutral-900 dark:text-neutral-100">
                {t("Inline")}
              </span>
              <span className="text-xs text-neutral-500">{t("Unified view")}</span>
            </Button>
          </div>
        </div>
      </div>
    </section>
  );
}
