import { useState, useRef, useEffect } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "./Button";
import { Input } from "./FormControls";
import { Dialog } from "./Overlay";

interface GoToLineDialogProps {
  totalLines: number;
  currentLine: number;
  onGo: (line: number) => void;
  onClose: () => void;
}

export function GoToLineDialog({ totalLines, currentLine, onGo, onClose }: GoToLineDialogProps) {
  const { t } = useTranslation();
  const [lineStr, setLineStr] = useState(String(currentLine));
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const line = parseInt(lineStr, 10);
    if (!isNaN(line) && line >= 1) {
      onGo(line);
      onClose();
    }
  };

  return (
    <Dialog.Root open onOpenChange={(open) => !open && onClose()}>
      <Dialog.Content className="ui-dialog-compact ui-dialog-line-content" showClose={false}>
        <Dialog.Header>
          <Dialog.Title>{t("Go to line")}</Dialog.Title>
        </Dialog.Header>
        <form onSubmit={handleSubmit}>
          <div className="flex items-center gap-1">
            <Input
              ref={inputRef}
              type="text"
              value={lineStr}
              onChange={(e) => {
                const val = e.target.value.replace(/\D/g, "");
                setLineStr(val);
              }}
              inputMode="numeric"
              autoComplete="off"
            />
            <span className="ui-dialog-suffix">/ {totalLines}</span>
          </div>
          <Dialog.Footer>
            <Dialog.Close asChild>
              <Button variant="ghost" size="sm" type="button">
                {t("Cancel")}
              </Button>
            </Dialog.Close>
            <Button variant="primary" size="sm" type="submit">
              {t("Go")}
            </Button>
          </Dialog.Footer>
        </form>
      </Dialog.Content>
    </Dialog.Root>
  );
}
