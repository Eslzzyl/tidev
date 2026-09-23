import { useTranslation } from "react-i18next";

import { Button } from "./Button";
import { Dialog } from "./Overlay";

interface InfoDialogProps {
  title: string;
  message: string;
  buttonText?: string;
  onClose: () => void;
}

export function InfoDialog({ title, message, buttonText, onClose }: InfoDialogProps) {
  const { t } = useTranslation();

  return (
    <Dialog.Root
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <Dialog.Content className="ui-dialog-compact" showClose={false}>
        <Dialog.Header>
          <Dialog.Title>{title}</Dialog.Title>
          <Dialog.Description>{message}</Dialog.Description>
        </Dialog.Header>
        <Dialog.Footer>
          <Dialog.Close asChild>
            <Button variant="primary">{buttonText ?? t("Got it")}</Button>
          </Dialog.Close>
        </Dialog.Footer>
      </Dialog.Content>
    </Dialog.Root>
  );
}
