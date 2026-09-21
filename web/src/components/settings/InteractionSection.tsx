import { useState } from "react";
import { ShieldAlert, AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import { useUIStore } from "../../stores/useUIStore";
import { Button } from "../ui";
import {
  checkNotificationAvailability,
  getNotificationPermission,
  requestNotificationPermission,
} from "../../utils/notifications";
import {
  SettingsSectionHeader,
  SettingsGroup,
  SettingsSwitchRow,
  SettingsSelectRow,
  SettingsActionRow,
} from "./SettingsCommon";

export function InteractionSection() {
  const { t } = useTranslation();
  const settings = useUIStore((s) => s.settings);
  const updateSettings = useUIStore((s) => s.updateSettings);

  const availability = checkNotificationAvailability();
  const [permission, setPermission] = useState(getNotificationPermission());
  const [requesting, setRequesting] = useState(false);

  return (
    <section className="space-y-6">
      <SettingsSectionHeader
        title={t("Interaction")}
        description={t("Customize how the chat input behaves")}
      />

      {/* Chat Input Group */}
      <SettingsGroup title={t("Chat Input")}>
        <SettingsSwitchRow
          label={t("Enter to send")}
          description={t("Press Enter to send, Shift+Enter for new line")}
          checked={settings.enterToSend}
          onCheckedChange={(checked) => updateSettings({ enterToSend: checked })}
        />
        <SettingsSwitchRow
          label={t("Fast mode")}
          description={t("Use the priority service tier for eligible GPT models")}
          checked={settings.fastMode}
          onCheckedChange={(checked) => {
            void api.setFastMode(checked).then((response) => {
              updateSettings({ fastMode: response.fast_mode });
            });
          }}
        />
      </SettingsGroup>

      {/* Desktop Notifications Group */}
      <SettingsGroup title={t("Desktop Notifications")}>
        {!availability.available ? (
          availability.reason === "insecure_context" ? (
            <div className="p-4 bg-amber-50/50 dark:bg-amber-950/20 text-xs">
              <div className="flex items-start gap-3">
                <ShieldAlert className="h-5 w-5 text-amber-600 dark:text-amber-400 shrink-0 mt-0.5" />
                <div className="space-y-1">
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
            <div className="p-4 bg-neutral-50/50 dark:bg-neutral-800/30 text-xs">
              <div className="flex items-start gap-3">
                <AlertTriangle className="h-5 w-5 text-neutral-500 shrink-0 mt-0.5" />
                <div className="space-y-1">
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
          <div className="p-4 bg-amber-50/50 dark:bg-amber-950/20 text-xs">
            <div className="flex items-start gap-3">
              <AlertTriangle className="h-5 w-5 text-amber-600 dark:text-amber-400 shrink-0 mt-0.5" />
              <div className="space-y-1">
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
          <SettingsActionRow
            label={t("Desktop Notifications")}
            description={t("Receive desktop notifications when tasks complete or need attention")}
            action={
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
            }
          />
        ) : (
          <>
            <SettingsSwitchRow
              label={t("Desktop Notifications")}
              description={t("Receive desktop notifications when tasks complete or need attention")}
              checked={settings.notificationEnabled}
              onCheckedChange={(checked) => updateSettings({ notificationEnabled: checked })}
            />
            {settings.notificationEnabled && (
              <SettingsSelectRow
                label={t("Notification trigger")}
                description={
                  settings.notificationCondition === "unfocused"
                    ? t("Only when window is unfocused")
                    : t("Always")
                }
                value={settings.notificationCondition}
                onValueChange={(val) =>
                  updateSettings({
                    notificationCondition: val as "unfocused" | "always",
                  })
                }
                options={[
                  {
                    value: "unfocused",
                    label: t("Only when window is unfocused"),
                  },
                  { value: "always", label: t("Always") },
                ]}
              />
            )}
          </>
        )}
      </SettingsGroup>
    </section>
  );
}
