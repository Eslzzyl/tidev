// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useUIStore } from "../stores/useUIStore";
import {
  checkNotificationAvailability,
  emitDesktopNotification,
  getNotificationPermission,
  requestNotificationPermission,
} from "./notifications";

describe("notifications utility", () => {
  const originalIsSecureContext = window.isSecureContext;
  const originalNotification = window.Notification;

  class MockNotification {
    static permission: NotificationPermission = "default";
    static requestPermission = vi.fn().mockResolvedValue("granted");

    title: string;
    body: string;
    tag: string;
    onclick: ((ev: Event) => void) | null = null;
    close = vi.fn();

    constructor(title: string, options: NotificationOptions = {}) {
      this.title = title;
      this.body = options.body ?? "";
      this.tag = options.tag ?? "";
    }
  }

  beforeEach(() => {
    Object.defineProperty(window, "isSecureContext", {
      value: true,
      configurable: true,
      writable: true,
    });
    // @ts-expect-error Mocking Notification constructor
    window.Notification = MockNotification;
    MockNotification.permission = "granted";
    useUIStore.getState().updateSettings({
      notificationEnabled: true,
      notificationCondition: "always",
    });
  });

  afterEach(() => {
    Object.defineProperty(window, "isSecureContext", {
      value: originalIsSecureContext,
      configurable: true,
      writable: true,
    });
    window.Notification = originalNotification;
    vi.restoreAllMocks();
  });

  describe("checkNotificationAvailability", () => {
    it("reports insecure_context when window.isSecureContext is false", () => {
      Object.defineProperty(window, "isSecureContext", {
        value: false,
        configurable: true,
        writable: true,
      });

      expect(checkNotificationAvailability()).toEqual({
        available: false,
        reason: "insecure_context",
      });
    });

    it("reports unsupported when Notification is not in window", () => {
      // @ts-expect-error Removing Notification for testing
      delete window.Notification;

      expect(checkNotificationAvailability()).toEqual({
        available: false,
        reason: "unsupported",
      });
    });

    it("reports available when secure and Notification exists", () => {
      expect(checkNotificationAvailability()).toEqual({ available: true });
    });
  });

  describe("permission helpers", () => {
    it("returns unsupported when insecure context", () => {
      Object.defineProperty(window, "isSecureContext", {
        value: false,
        configurable: true,
        writable: true,
      });
      expect(getNotificationPermission()).toBe("unsupported");
    });

    it("returns current permission when available", () => {
      MockNotification.permission = "denied";
      expect(getNotificationPermission()).toBe("denied");

      MockNotification.permission = "granted";
      expect(getNotificationPermission()).toBe("granted");
    });

    it("requests permission when available", async () => {
      const permission = await requestNotificationPermission();
      expect(permission).toBe("granted");
      expect(MockNotification.requestPermission).toHaveBeenCalled();
    });
  });

  describe("emitDesktopNotification", () => {
    it("does nothing if notificationEnabled is false", () => {
      useUIStore.getState().updateSettings({ notificationEnabled: false });

      const notification = emitDesktopNotification({
        title: "tidev",
        body: "Response complete",
      });
      expect(notification).toBeNull();
    });

    it("does nothing if permission is not granted", () => {
      MockNotification.permission = "denied";

      const notification = emitDesktopNotification({
        title: "tidev",
        body: "Response complete",
      });
      expect(notification).toBeNull();
    });

    it("skips emission when condition is unfocused but page is focused", () => {
      useUIStore.getState().updateSettings({
        notificationEnabled: true,
        notificationCondition: "unfocused",
      });

      Object.defineProperty(document, "visibilityState", {
        value: "visible",
        configurable: true,
        writable: true,
      });
      vi.spyOn(document, "hasFocus").mockReturnValue(true);

      const notification = emitDesktopNotification({
        title: "tidev",
        body: "Response complete",
      });
      expect(notification).toBeNull();
    });

    it("emits notification when condition is unfocused and page is hidden", () => {
      useUIStore.getState().updateSettings({
        notificationEnabled: true,
        notificationCondition: "unfocused",
      });

      Object.defineProperty(document, "visibilityState", {
        value: "hidden",
        configurable: true,
        writable: true,
      });

      const notification = emitDesktopNotification({
        title: "tidev",
        body: "Response complete",
        tag: "test-tag",
      });
      expect(notification).not.toBeNull();
      expect(notification?.title).toBe("tidev");
      expect(notification?.body).toBe("Response complete");
      expect(notification?.tag).toBe("test-tag");
    });

    it("triggers onClick and window.focus when notification is clicked", () => {
      const onClick = vi.fn();
      const focusSpy = vi.spyOn(window, "focus").mockImplementation(() => {});

      const notification = emitDesktopNotification({
        title: "tidev",
        body: "Response complete",
        onClick,
      });

      expect(notification).not.toBeNull();
      notification?.onclick?.(new Event("click") as unknown as MouseEvent);

      expect(focusSpy).toHaveBeenCalled();
      expect(onClick).toHaveBeenCalled();
      expect(notification?.close).toHaveBeenCalled();
    });
  });
});
