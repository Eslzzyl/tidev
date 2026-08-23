import { useState } from "react";
import { Check, Clipboard } from "lucide-react";
import { useTranslation } from "react-i18next";

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
    <button
      onClick={handleCopy}
      className="ml-2 rounded p-1 text-neutral-400 transition-colors hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300"
      title={t("Copy to clipboard")}
    >
      {copied ? <Check className="h-4 w-4" /> : <Clipboard className="h-4 w-4" />}
    </button>
  );
}
