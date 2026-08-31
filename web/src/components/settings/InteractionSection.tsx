import { CornerDownLeft } from "lucide-react";
import { useUIStore } from "../../stores/useUIStore";
import { useTranslation } from "react-i18next";
import { Switch } from "../ui";

export function InteractionSection() {
  const { t } = useTranslation();
  const enterToSend = useUIStore((s) => s.settings.enterToSend);
  const updateSettings = useUIStore((s) => s.updateSettings);

  return (
    <section className="space-y-6">
      <div>
        <h2 className="text-base font-semibold text-neutral-900 dark:text-neutral-100">
          {t("Interaction")}
        </h2>
        <p className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">
          {t("Customize how the chat input behaves")}
        </p>
      </div>

      <div className="space-y-3">
        <label className="text-xs font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
          {t("Chat Input")}
        </label>
        <div className="rounded-xl border border-neutral-200/80 bg-neutral-50/50 p-4 dark:border-neutral-800/80 dark:bg-neutral-800/30">
          <label className="flex items-center justify-between gap-4 cursor-pointer">
            <div className="flex items-center gap-3 min-w-0">
              <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-white shadow-xs text-neutral-600 dark:bg-neutral-800 dark:text-neutral-300">
                <CornerDownLeft className="h-4 w-4" />
              </div>
              <div className="min-w-0">
                <span className="block truncate text-xs font-medium text-neutral-900 dark:text-neutral-100">
                  {t("Enter to send")}
                </span>
                <span className="block text-[11px] text-neutral-500 dark:text-neutral-400">
                  {t("Press Enter to send, Shift+Enter for new line")}
                </span>
              </div>
            </div>
            <Switch
              aria-label={t("Enter to send")}
              checked={enterToSend}
              onCheckedChange={(checked) => updateSettings({ enterToSend: checked })}
            />
          </label>
        </div>
      </div>
    </section>
  );
}
