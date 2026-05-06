import { X, CheckCircle, AlertCircle, Info, AlertTriangle } from "lucide-react";
import { useToastStore, type ToastType } from "../../stores/useToastStore";

const iconMap: Record<ToastType, React.ReactNode> = {
  success: <CheckCircle className="h-4 w-4 text-green-500" />,
  error: <AlertCircle className="h-4 w-4 text-red-500" />,
  warning: <AlertTriangle className="h-4 w-4 text-amber-500" />,
  info: <Info className="h-4 w-4 text-blue-500" />,
};

const bgMap: Record<ToastType, string> = {
  success:
    "border-green-200 bg-green-50 dark:border-green-900 dark:bg-green-950",
  error: "border-red-200 bg-red-50 dark:border-red-900 dark:bg-red-950",
  warning:
    "border-amber-200 bg-amber-50 dark:border-amber-900 dark:bg-amber-950",
  info: "border-blue-200 bg-blue-50 dark:border-blue-900 dark:bg-blue-950",
};

const textMap: Record<ToastType, string> = {
  success: "text-green-800 dark:text-green-200",
  error: "text-red-800 dark:text-red-200",
  warning: "text-amber-800 dark:text-amber-200",
  info: "text-blue-800 dark:text-blue-200",
};

export function ToastContainer() {
  const toasts = useToastStore((s) => s.toasts);
  const removeToast = useToastStore((s) => s.removeToast);

  if (toasts.length === 0) return null;

  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-[9999] flex flex-col gap-2">
      {toasts.map((toast) => (
        <div
          key={toast.id}
          className={`pointer-events-auto flex items-center gap-2 rounded-lg border px-3 py-2 shadow-lg transition-all duration-300 ${
            bgMap[toast.type]
          } ${textMap[toast.type]}`}
          style={{
            minWidth: "200px",
            maxWidth: "400px",
            animation: "toast-slide-in 0.2s ease-out",
          }}
        >
          {iconMap[toast.type]}
          <span className="flex-1 text-xs font-medium">{toast.message}</span>
          <button
            onClick={() => removeToast(toast.id)}
            className="ml-1 shrink-0 rounded p-0.5 opacity-60 hover:opacity-100"
          >
            <X className="h-3 w-3" />
          </button>
        </div>
      ))}
    </div>
  );
}
