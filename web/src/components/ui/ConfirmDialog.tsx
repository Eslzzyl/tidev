import { AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "./Button";
import { Dialog } from "./Overlay";

interface ConfirmDialogProps {
  isOpen?: boolean;
  title: string;
  message: string;
  confirmText?: string;
  cancelText?: string;
  confirmLabel?: string;
  danger?: boolean;
  isLoading?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog({
  isOpen = true,
  title,
  message,
  confirmText,
  cancelText,
  confirmLabel,
  danger = false,
  isLoading = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const { t } = useTranslation();
  if (!isOpen) return null;

  const confirmBtnLabel = confirmText || confirmLabel || t("Confirm");
  const cancelBtnLabel = cancelText || t("Cancel");

  return (
    <Dialog.Root
      open={isOpen}
      onOpenChange={(open) => {
        if (!open && !isLoading) onCancel();
      }}
    >
      <Dialog.Content className="ui-dialog-compact" showClose={false}>
        <Dialog.Header>
          <Dialog.Title className="ui-dialog-title-with-icon">
            {danger && <AlertTriangle className="ui-dialog-danger-icon" />}
            {title}
          </Dialog.Title>
          <Dialog.Description>{message}</Dialog.Description>
        </Dialog.Header>
        <Dialog.Footer>
          <Dialog.Close asChild>
            <Button variant="ghost" disabled={isLoading}>
              {cancelBtnLabel}
            </Button>
          </Dialog.Close>
          <Button variant={danger ? "danger" : "primary"} onClick={onConfirm} loading={isLoading}>
            {confirmBtnLabel}
          </Button>
        </Dialog.Footer>
      </Dialog.Content>
    </Dialog.Root>
  );
}
