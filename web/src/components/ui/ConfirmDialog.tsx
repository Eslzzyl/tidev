import { AlertTriangle, Loader2 } from "lucide-react";

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
  if (!isOpen) return null;

  const confirmBtnLabel = confirmText || confirmLabel || "Confirm";
  const cancelBtnLabel = cancelText || "Cancel";

  return (
    <div
      className="fixed inset-0 z-[9998] flex items-center justify-center bg-black/30"
      onClick={(e) => {
        if (e.target === e.currentTarget && !isLoading) onCancel();
      }}
    >
      <div className="w-72 rounded-lg border border-neutral-200 bg-white p-4 shadow-xl dark:border-neutral-700 dark:bg-neutral-900">
        <div className="mb-3 flex items-center gap-2">
          {danger && <AlertTriangle className="h-4 w-4 text-red-500" />}
          <span className="text-sm font-medium text-neutral-800 dark:text-neutral-200">
            {title}
          </span>
        </div>
        <p className="mb-4 text-xs text-neutral-600 dark:text-neutral-400">
          {message}
        </p>
        <div className="flex justify-end gap-2">
          <button
            onClick={onCancel}
            disabled={isLoading}
            className="rounded px-3 py-1 text-xs text-neutral-600 hover:bg-neutral-100 disabled:opacity-40 dark:text-neutral-400 dark:hover:bg-neutral-800"
          >
            {cancelBtnLabel}
          </button>
          <button
            onClick={onConfirm}
            disabled={isLoading}
            className={`flex items-center gap-1 rounded px-3 py-1 text-xs text-white ${
              danger
                ? "bg-red-600 hover:bg-red-700"
                : "bg-blue-600 hover:bg-blue-700"
            } disabled:opacity-40`}
          >
            {isLoading && <Loader2 className="h-3 w-3 animate-spin" />}
            {confirmBtnLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
