import { useUIStore } from "../../stores/useUIStore";
import { useTranslation } from "react-i18next";
import { Switch } from "../ui";

export function InteractionSection() {
  const { t } = useTranslation();
  const enterToSend = useUIStore((s) => s.settings.enterToSend);
  const updateSettings = useUIStore((s) => s.updateSettings);

  return (
    <section>
      <h2 className="mb-1 text-sm font-medium text-neutral-900 dark:text-neutral-100">
        {t("Interaction")}
      </h2>
      <p className="mb-4 text-sm text-neutral-500 dark:text-neutral-400">
        {t("Customize how the chat input behaves")}
      </p>

      <label className="flex items-center justify-between rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
        <div>
          <span className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
            {t("Enter to send")}
          </span>
          <p className="text-xs text-neutral-500">
            {t("Press Enter to send, Shift+Enter for new line")}
          </p>
        </div>
        <Switch
          aria-label={t("Enter to send")}
          checked={enterToSend}
          onCheckedChange={(checked) => updateSettings({ enterToSend: checked })}
        />
      </label>
    </section>
  );
}
