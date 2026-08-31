import { useState, useRef, useEffect } from "react";
import { File, Folder } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "./Button";
import { Input } from "./FormControls";
import { Dialog } from "./Overlay";

interface CreateItemDialogProps {
  /** Parent directory path where the item will be created */
  parentPath: string;
  /** Type of item to create */
  type: "file" | "directory";
  /** Called with the new item name */
  onSubmit: (name: string) => void;
  /** Called to close the dialog */
  onClose: () => void;
}

export function CreateItemDialog({ parentPath, type, onSubmit, onClose }: CreateItemDialogProps) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = name.trim();
    if (trimmed) {
      onSubmit(trimmed);
    }
  };

  return (
    <Dialog.Root open onOpenChange={(open) => !open && onClose()}>
      <Dialog.Content className="ui-dialog-compact">
        <Dialog.Header>
          <div className="ui-dialog-title-with-icon">
            {type === "file" ? (
              <File className="ui-dialog-file-icon" />
            ) : (
              <Folder className="ui-dialog-directory-icon" />
            )}
            <Dialog.Title>{type === "file" ? t("New File") : t("New Directory")}</Dialog.Title>
          </div>
        </Dialog.Header>

        <Dialog.Description>
          {t("in")} <span className="font-mono">{parentPath || "/"}</span>
        </Dialog.Description>

        <form onSubmit={handleSubmit}>
          <Input
            ref={inputRef}
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={type === "file" ? t("filename.ext") : t("directory-name")}
            autoComplete="off"
          />
          <Dialog.Footer>
            <Dialog.Close asChild>
              <Button variant="ghost" type="button">
                {t("Cancel")}
              </Button>
            </Dialog.Close>
            <Button variant="primary" type="submit" disabled={!name.trim()}>
              {t("Create")}
            </Button>
          </Dialog.Footer>
        </form>
      </Dialog.Content>
    </Dialog.Root>
  );
}
