import { useLayoutEffect, useRef, useState } from "react";
import { X, CheckCircle, AlertCircle, Info, AlertTriangle } from "lucide-react";
import { useToastStore, type Toast } from "../../stores/useToastStore";

export type { ToastType } from "../../stores/useToastStore";

const iconMap: Record<string, React.ReactNode> = {
  success: <CheckCircle className="h-4 w-4 text-green-500" />,
  error: <AlertCircle className="h-4 w-4 text-red-500" />,
  warning: <AlertTriangle className="h-4 w-4 text-amber-500" />,
  info: <Info className="h-4 w-4 text-blue-500" />,
};

const bgMap: Record<string, string> = {
  success:
    "border-green-200 bg-green-50 dark:border-green-900 dark:bg-green-950",
  error: "border-red-200 bg-red-50 dark:border-red-900 dark:bg-red-950",
  warning:
    "border-amber-200 bg-amber-50 dark:border-amber-900 dark:bg-amber-950",
  info: "border-blue-200 bg-blue-50 dark:border-blue-900 dark:bg-blue-950",
};

const textMap: Record<string, string> = {
  success: "text-green-800 dark:text-green-200",
  error: "text-red-800 dark:text-red-200",
  warning: "text-amber-800 dark:text-amber-200",
  info: "text-blue-800 dark:text-blue-200",
};

export function ToastContainer() {
  const storeToasts = useToastStore((s) => s.toasts);
  const removeToast = useToastStore((s) => s.removeToast);

  // Cache of toasts that have been removed from the store but are still
  // playing their exit animation.  We keep a snapshot so we can still render
  // the toast content during the 200 ms exit.
  const [exitingToasts, setExitingToasts] = useState<Toast[]>([]);
  const prevToastsRef = useRef<Toast[]>([]);

  // Synchronously detect toasts that disappeared from the store so there is
  // zero frames where the toast is gone without an exit animation.
  useLayoutEffect(() => {
    const prevToasts = prevToastsRef.current;
    const currentIds = new Set(storeToasts.map((t) => t.id));

    for (const prev of prevToasts) {
      if (!currentIds.has(prev.id)) {
        // Store removed this toast – capture a snapshot for the exit animation.
        setExitingToasts((prevList) => [...prevList, prev]);
        setTimeout(() => {
          setExitingToasts((prevList) =>
            prevList.filter((t) => t.id !== prev.id),
          );
        }, 200);
      }
    }

    prevToastsRef.current = storeToasts;
  }, [storeToasts]);

  // Merge active + exiting toasts.  Exiting duplicates are discarded.
  const activeIds = new Set(storeToasts.map((t) => t.id));
  const allToasts = [
    ...storeToasts,
    ...exitingToasts.filter((t) => !activeIds.has(t.id)),
  ];

  if (allToasts.length === 0) return null;

  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-[9999] flex flex-col gap-2">
      {allToasts.map((toast) => {
        const isExiting = !activeIds.has(toast.id);
        return (
          <div
            key={toast.id}
            className={`pointer-events-auto flex items-center gap-2 rounded-lg border px-3 py-2 shadow-lg motion-safe:transition-all motion-safe:duration-200 motion-safe:ease-smooth ${
              bgMap[toast.type]
            } ${textMap[toast.type]} ${
              isExiting
                ? "motion-safe:animate-toast-out"
                : "motion-safe:animate-toast-in"
            }`}
            style={{
              minWidth: "200px",
              maxWidth: "400px",
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
        );
      })}
    </div>
  );
}
