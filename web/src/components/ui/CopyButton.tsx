import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { useTranslation } from "react-i18next";
import { IconButton } from "./Button";

interface CopyButtonProps {
  content: string;
}

export function CopyButton({ content }: CopyButtonProps) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(content);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error("Failed to copy:", err);
    }
  }

  return (
    <IconButton
      label={t("Copy to clipboard")}
      size="sm"
      type="button"
      onClick={handleCopy}
      className="message-action"
      title={t("Copy to clipboard")}
    >
      {copied ? <Check size={17} /> : <Copy size={17} />}
    </IconButton>
  );
}
