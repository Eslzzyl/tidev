import { create } from "zustand";
import type { AppEvent } from "../types/events";

export interface PendingPermission {
  id: string;
  toolCallId: string;
  toolName: string;
  arguments: string;
  sessionId: string;
  requestId: number;
}

interface PermissionState {
  pendingPermissions: PendingPermission[];
  autoAccept: Record<string, boolean>; // sessionId -> auto-accept
}

interface PermissionActions {
  addPermission: (permission: PendingPermission) => void;
  removePermission: (id: string) => void;
  clearSessionPermissions: (sessionId: string) => void;
  setAutoAccept: (sessionId: string, enabled: boolean) => void;
  isAutoAccepting: (sessionId: string) => boolean;
  handlePermissionRequestEvent: (event: AppEvent) => void;
}

const loadAutoAccept = (): Record<string, boolean> => {
  try {
    const raw = localStorage.getItem("permission.autoAccept");
    return raw ? JSON.parse(raw) : {};
  } catch {
    return {};
  }
};

export const usePermissionStore = create<PermissionState & PermissionActions>(
  (set, get) => ({
    pendingPermissions: [],
    autoAccept: loadAutoAccept(),

    addPermission: (permission) =>
      set((state) => ({
        pendingPermissions: [...state.pendingPermissions, permission],
      })),

    removePermission: (id) =>
      set((state) => ({
        pendingPermissions: state.pendingPermissions.filter((p) => p.id !== id),
      })),

    clearSessionPermissions: (sessionId) =>
      set((state) => ({
        pendingPermissions: state.pendingPermissions.filter(
          (p) => p.sessionId !== sessionId,
        ),
      })),

    setAutoAccept: (sessionId, enabled) => {
      const newAutoAccept = { ...get().autoAccept, [sessionId]: enabled };
      localStorage.setItem(
        "permission.autoAccept",
        JSON.stringify(newAutoAccept),
      );
      set({ autoAccept: newAutoAccept });
    },

    isAutoAccepting: (sessionId) => {
      return get().autoAccept[sessionId] ?? false;
    },

    handlePermissionRequestEvent: (event) => {
      if (event.type !== "permission_request") return;
      const permission: PendingPermission = {
        id: `${event.session_id}-${event.tool_call_id}`,
        toolCallId: event.tool_call_id,
        toolName: event.tool_name,
        arguments: event.arguments,
        sessionId: event.session_id,
        requestId: event.request_id,
      };

      // Check auto-accept
      if (get().isAutoAccepting(event.session_id)) {
        // Auto-approved — no UI needed
        return;
      }

      set((state) => ({
        pendingPermissions: [...state.pendingPermissions, permission],
      }));
    },
  }),
);
