import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "./ui/ConfirmDialog";

interface StartupProviderDialogProps {
  onOpenSettings: () => void;
  onDismiss: () => void;
}

export function StartupProviderDialog({ onOpenSettings, onDismiss }: StartupProviderDialogProps) {
  const { t } = useTranslation();

  return (
    <ConfirmDialog
      title={t("Connect a provider")}
      message={t("Connect a provider in Settings to start using tidev.")}
      confirmText={t("Open provider settings")}
      cancelText={t("Later")}
      onConfirm={onOpenSettings}
      onCancel={onDismiss}
    />
  );
}
