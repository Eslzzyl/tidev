import { create } from "zustand";

export type ToastType = "success" | "error" | "info" | "warning";

export interface Toast {
  id: string;
  message: string;
  type: ToastType;
  duration: number;
}

interface ToastStore {
  toasts: Toast[];
  addToast: (message: string, type?: ToastType, duration?: number) => void;
  removeToast: (id: string) => void;
}

let toastCounter = 0;

export const useToastStore = create<ToastStore>((set, get) => ({
  toasts: [],

  addToast: (message, type = "info", duration = 3000) => {
    const id = `toast-${++toastCounter}`;
    const toast: Toast = { id, message, type, duration };

    set((s) => ({ toasts: [...s.toasts, toast] }));

    // Auto-remove after duration
    if (duration > 0) {
      setTimeout(() => {
        get().removeToast(id);
      }, duration);
    }
  },

  removeToast: (id) => {
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
  },
}));

/**
 * Convenience functions for common toast types.
 */
export const toast = {
  success: (message: string) =>
    useToastStore.getState().addToast(message, "success"),
  error: (message: string) =>
    useToastStore.getState().addToast(message, "error", 5000),
  info: (message: string) => useToastStore.getState().addToast(message, "info"),
  warning: (message: string) =>
    useToastStore.getState().addToast(message, "warning", 4000),
};
