import { useUIStore } from "../stores/useUIStore";

export type NotificationAvailability =
  | { available: true }
  | { available: false; reason: "insecure_context" | "unsupported" };

/**
 * Checks whether desktop notifications can be used in the current browser environment.
 * Verifies both secure context (HTTPS/localhost) and Notification API availability.
 */
export function checkNotificationAvailability(): NotificationAvailability {
  if (typeof window === "undefined") {
    return { available: false, reason: "unsupported" };
  }
  // Modern browsers require a secure context (https or localhost/127.0.0.1) for notifications.
  if (window.isSecureContext === false) {
    return { available: false, reason: "insecure_context" };
  }
  if (!("Notification" in window)) {
    return { available: false, reason: "unsupported" };
  }
  return { available: true };
}

/**
 * Returns the current notification permission or 'unsupported'.
 */
export function getNotificationPermission(): NotificationPermission | "unsupported" {
  const availability = checkNotificationAvailability();
  if (!availability.available) {
    return "unsupported";
  }
  return Notification.permission;
}

/**
 * Requests notification permission from the user.
 * Must be triggered by a user gesture.
 */
export async function requestNotificationPermission(): Promise<
  NotificationPermission | "unsupported"
> {
  const availability = checkNotificationAvailability();
  if (!availability.available) {
    return "unsupported";
  }
  try {
    return await Notification.requestPermission();
  } catch {
    return Notification.permission;
  }
}

export interface DesktopNotificationOptions {
  title: string;
  body?: string;
  tag?: string;
  onClick?: () => void;
}

/**
 * Emits a desktop notification if conditions (enabled, secure context, permission, unfocused) are met.
 */
export function emitDesktopNotification(options: DesktopNotificationOptions): Notification | null {
  const availability = checkNotificationAvailability();
  if (!availability.available) {
    return null;
  }
  if (Notification.permission !== "granted") {
    return null;
  }

  const { notificationEnabled, notificationCondition } = useUIStore.getState().settings;
  if (!notificationEnabled) {
    return null;
  }

  // TUI-aligned condition check: "unfocused" (default) requires the window/tab to be inactive.
  if (notificationCondition === "unfocused") {
    const isUnfocused =
      typeof document !== "undefined" &&
      (document.visibilityState === "hidden" ||
        (typeof document.hasFocus === "function" && !document.hasFocus()));
    if (!isUnfocused) {
      return null;
    }
  }

  try {
    const notification = new Notification(options.title, {
      body: options.body,
      tag: options.tag,
      icon: "/favicon.ico",
    });

    notification.onclick = () => {
      try {
        window.focus();
      } catch {
        // Ignore focus errors
      }
      options.onClick?.();
      notification.close();
    };

    return notification;
  } catch {
    return null;
  }
}
