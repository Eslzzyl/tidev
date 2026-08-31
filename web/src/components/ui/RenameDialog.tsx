import { useState, useRef, useEffect } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "./Button";
import { Input } from "./FormControls";
import { Dialog } from "./Overlay";

interface RenameDialogProps {
  currentName: string;
  onSubmit: (newName: string) => void;
  onClose: () => void;
}

export function RenameDialog({ currentName, onSubmit, onClose }: RenameDialogProps) {
  const { t } = useTranslation();
  const [name, setName] = useState(currentName);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    // Select the name without extension for files
    const dotIndex = currentName.lastIndexOf(".");
    if (dotIndex > 0) {
      inputRef.current?.setSelectionRange(0, dotIndex);
    } else {
      inputRef.current?.select();
    }
  }, [currentName]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = name.trim();
    if (trimmed && trimmed !== currentName) {
      onSubmit(trimmed);
    } else {
      onClose();
    }
  };

  return (
    <Dialog.Root open onOpenChange={(open) => !open && onClose()}>
      <Dialog.Content className="ui-dialog-compact" showClose={false}>
        <Dialog.Header>
          <Dialog.Title>{t("Rename")}</Dialog.Title>
        </Dialog.Header>
        <form onSubmit={handleSubmit}>
          <Input
            ref={inputRef}
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoComplete="off"
          />
          <Dialog.Footer>
            <Dialog.Close asChild>
              <Button variant="ghost" type="button">
                {t("Cancel")}
              </Button>
            </Dialog.Close>
            <Button
              type="submit"
              variant="primary"
              disabled={!name.trim() || name.trim() === currentName}
            >
              {t("Rename")}
            </Button>
          </Dialog.Footer>
        </form>
      </Dialog.Content>
    </Dialog.Root>
  );
}
