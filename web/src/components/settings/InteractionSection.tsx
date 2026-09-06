import { useState } from "react";
import { CornerDownLeft, Bell, AlertTriangle, ShieldAlert } from "lucide-react";
import { useUIStore } from "../../stores/useUIStore";
import { useTranslation } from "react-i18next";
import { Switch, Button, Select } from "../ui";
import {
  checkNotificationAvailability,
  getNotificationPermission,
  requestNotificationPermission,
} from "../../utils/notifications";

export function InteractionSection() {
  const { t } = useTranslation();
  const settings = useUIStore((s) => s.settings);
  const updateSettings = useUIStore((s) => s.updateSettings);

  const availability = checkNotificationAvailability();
  const [permission, setPermission] = useState(getNotificationPermission());
  const [requesting, setRequesting] = useState(false);

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

      {/* Chat Input Group */}
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
              checked={settings.enterToSend}
              onCheckedChange={(checked) => updateSettings({ enterToSend: checked })}
            />
          </label>
        </div>
      </div>

      {/* Notifications Group */}
      <div className="space-y-3">
        <label className="text-xs font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
          {t("Desktop Notifications")}
        </label>

        {!availability.available ? (
          availability.reason === "insecure_context" ? (
            <div className="rounded-xl border border-amber-200/80 bg-amber-50/50 p-4 dark:border-amber-900/60 dark:bg-amber-950/20">
              <div className="flex items-start gap-3">
                <ShieldAlert className="h-5 w-5 text-amber-600 dark:text-amber-400 shrink-0 mt-0.5" />
                <div className="space-y-1 text-xs">
                  <span className="font-medium text-amber-900 dark:text-amber-200 block">
                    {t("Unavailable in non-secure context")}
                  </span>
                  <p className="text-amber-700 dark:text-amber-300 leading-relaxed text-[11px]">
                    {t(
                      "Desktop notifications require a secure context (HTTPS or localhost/127.0.0.1). When accessing via LAN HTTP, browser security disables notifications.",
                    )}
                  </p>
                </div>
              </div>
            </div>
          ) : (
            <div className="rounded-xl border border-neutral-200/80 bg-neutral-50/50 p-4 dark:border-neutral-800/80 dark:bg-neutral-800/30">
              <div className="flex items-start gap-3">
                <AlertTriangle className="h-5 w-5 text-neutral-500 shrink-0 mt-0.5" />
                <div className="space-y-1 text-xs">
                  <span className="font-medium text-neutral-900 dark:text-neutral-100 block">
                    {t("Desktop Notifications")}
                  </span>
                  <p className="text-neutral-500 dark:text-neutral-400 text-[11px]">
                    {t("Desktop notifications are not supported by this browser.")}
                  </p>
                </div>
              </div>
            </div>
          )
        ) : permission === "denied" ? (
          <div className="rounded-xl border border-amber-200/80 bg-amber-50/50 p-4 dark:border-amber-900/60 dark:bg-amber-950/20">
            <div className="flex items-start gap-3">
              <AlertTriangle className="h-5 w-5 text-amber-600 dark:text-amber-400 shrink-0 mt-0.5" />
              <div className="space-y-1 text-xs">
                <span className="font-medium text-amber-900 dark:text-amber-200 block">
                  {t("Notifications blocked")}
                </span>
                <p className="text-amber-700 dark:text-amber-300 leading-relaxed text-[11px]">
                  {t(
                    "Notifications are blocked by your browser. Please allow notifications in site settings.",
                  )}
                </p>
              </div>
            </div>
          </div>
        ) : permission === "default" ? (
          <div className="rounded-xl border border-neutral-200/80 bg-neutral-50/50 p-4 dark:border-neutral-800/80 dark:bg-neutral-800/30">
            <div className="flex items-center justify-between gap-4">
              <div className="flex items-center gap-3 min-w-0">
                <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-white shadow-xs text-neutral-600 dark:bg-neutral-800 dark:text-neutral-300">
                  <Bell className="h-4 w-4" />
                </div>
                <div className="min-w-0">
                  <span className="block truncate text-xs font-medium text-neutral-900 dark:text-neutral-100">
                    {t("Desktop Notifications")}
                  </span>
                  <span className="block text-[11px] text-neutral-500 dark:text-neutral-400">
                    {t("Receive desktop notifications when tasks complete or need attention")}
                  </span>
                </div>
              </div>
              <Button
                size="sm"
                variant="secondary"
                loading={requesting}
                onClick={async () => {
                  setRequesting(true);
                  try {
                    const res = await requestNotificationPermission();
                    setPermission(res);
                  } finally {
                    setRequesting(false);
                  }
                }}
              >
                {t("Enable notifications")}
              </Button>
            </div>
          </div>
        ) : (
          <div className="rounded-xl border border-neutral-200/80 bg-neutral-50/50 p-4 divide-y divide-neutral-200/60 dark:border-neutral-800/80 dark:bg-neutral-800/30 dark:divide-neutral-800/60">
            <label className="flex items-center justify-between gap-4 cursor-pointer pb-3.5">
              <div className="flex items-center gap-3 min-w-0">
                <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-white shadow-xs text-neutral-600 dark:bg-neutral-800 dark:text-neutral-300">
                  <Bell className="h-4 w-4" />
                </div>
                <div className="min-w-0">
                  <span className="block truncate text-xs font-medium text-neutral-900 dark:text-neutral-100">
                    {t("Desktop Notifications")}
                  </span>
                  <span className="block text-[11px] text-neutral-500 dark:text-neutral-400">
                    {t("Receive desktop notifications when tasks complete or need attention")}
                  </span>
                </div>
              </div>
              <Switch
                aria-label={t("Desktop Notifications")}
                checked={settings.notificationEnabled}
                onCheckedChange={(checked) => updateSettings({ notificationEnabled: checked })}
              />
            </label>

            {settings.notificationEnabled && (
              <div className="pt-3.5 flex items-center justify-between gap-4">
                <div className="min-w-0">
                  <span className="block truncate text-xs font-medium text-neutral-900 dark:text-neutral-100">
                    {t("Notification trigger")}
                  </span>
                  <span className="block text-[11px] text-neutral-500 dark:text-neutral-400">
                    {settings.notificationCondition === "unfocused"
                      ? t("Only when window is unfocused")
                      : t("Always")}
                  </span>
                </div>
                <Select
                  value={settings.notificationCondition}
                  onValueChange={(val) =>
                    updateSettings({ notificationCondition: val as "unfocused" | "always" })
                  }
                  ariaLabel={t("Notification trigger")}
                  options={[
                    { value: "unfocused", label: t("Only when window is unfocused") },
                    { value: "always", label: t("Always") },
                  ]}
                />
              </div>
            )}
          </div>
        )}
      </div>
    </section>
  );
}
