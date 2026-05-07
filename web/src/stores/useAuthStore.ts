/**
 * Auth store for managing web UI authentication.
 *
 * - Token is stored in localStorage under "web_auth_token".
 * - All /api/* requests include `Authorization: Bearer <token>`.
 * - SSE connections pass token via `?token=` query parameter.
 */
import { create } from "zustand";

const AUTH_TOKEN_KEY = "web_auth_token";
const API_BASE = "/api";

export interface AuthState {
  /** The stored auth token, or null if none */
  token: string | null;
  /** Whether the user has provided a valid token */
  isAuthenticated: boolean;
  /** Whether the backend requires authentication */
  isAuthRequired: boolean;
  /** Whether we're checking auth status */
  isLoading: boolean;
  /** Error message from last operation */
  error: string | null;
}

export interface AuthActions {
  /** Check if backend requires auth and if our stored token is valid */
  checkAuthStatus: () => Promise<void>;
  /** Verify a token with the backend */
  verifyToken: (token: string) => Promise<boolean>;
  /** Store token locally (does not verify) */
  setToken: (token: string) => void;
  /** Clear stored token */
  clearToken: () => void;
  /** Configure/set a new web auth token on the backend */
  configureToken: (newToken: string) => Promise<boolean>;
  /** Clear error */
  clearError: () => void;
}

type AuthStore = AuthState & AuthActions;

export const useAuthStore = create<AuthStore>((set, get) => ({
  token: loadToken(),
  isAuthenticated: !!loadToken(),
  isAuthRequired: false,
  isLoading: true,
  error: null,

  checkAuthStatus: async () => {
    set({ isLoading: true, error: null });
    try {
      const res = await fetch(`${API_BASE}/auth/status`);
      if (!res.ok) {
        // If we can't reach the backend, consider auth not required
        set({ isAuthRequired: false, isLoading: false });
        return;
      }
      const data: { auth_required: boolean } = await res.json();
      const token = get().token;

      if (!data.auth_required) {
        // Auth not required — all good
        set({
          isAuthRequired: false,
          isAuthenticated: true,
          isLoading: false,
        });
        return;
      }

      // Auth is required — verify our stored token
      if (token) {
        const valid = await get().verifyToken(token);
        set({
          isAuthRequired: true,
          isAuthenticated: valid,
          isLoading: false,
          error: valid ? null : "Stored token is invalid. Please re-enter.",
        });
        if (!valid) {
          clearStoredToken();
        }
      } else {
        set({
          isAuthRequired: true,
          isAuthenticated: false,
          isLoading: false,
        });
      }
    } catch {
      // Cannot reach backend — backend might not be running
      set({ isAuthRequired: false, isAuthenticated: true, isLoading: false });
    }
  },

  verifyToken: async (token: string): Promise<boolean> => {
    try {
      const res = await fetch(`${API_BASE}/auth/verify`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ token }),
      });
      if (!res.ok) return false;
      const data: { valid: boolean } = await res.json();
      return data.valid;
    } catch {
      return false;
    }
  },

  setToken: (token: string) => {
    saveToken(token);
    set({ token, isAuthenticated: true, error: null });
  },

  clearToken: () => {
    clearStoredToken();
    set({ token: null, isAuthenticated: false });
  },

  configureToken: async (newToken: string): Promise<boolean> => {
    set({ error: null });
    try {
      const token = get().token;
      const headers: Record<string, string> = {
        "Content-Type": "application/json",
      };
      if (token) {
        headers["Authorization"] = `Bearer ${token}`;
      }

      const res = await fetch(`${API_BASE}/auth/configure`, {
        method: "POST",
        headers,
        body: JSON.stringify({ new_token: newToken }),
      });

      if (!res.ok) {
        const err = await res.json().catch(() => ({ error: "Unknown error" }));
        set({ error: err.error || "Failed to configure token" });
        return false;
      }

      // Update local token
      if (newToken) {
        saveToken(newToken);
        set({ token: newToken, isAuthenticated: true, error: null });
      } else {
        clearStoredToken();
        set({ token: null, isAuthenticated: false, error: null });
      }
      return true;
    } catch (err) {
      set({ error: err instanceof Error ? err.message : "Network error" });
      return false;
    }
  },

  clearError: () => set({ error: null }),
}));

function loadToken(): string | null {
  try {
    return localStorage.getItem(AUTH_TOKEN_KEY);
  } catch {
    return null;
  }
}

function saveToken(token: string) {
  try {
    localStorage.setItem(AUTH_TOKEN_KEY, token);
  } catch {
    // localStorage may be unavailable
  }
}

function clearStoredToken() {
  try {
    localStorage.removeItem(AUTH_TOKEN_KEY);
  } catch {
    // localStorage may be unavailable
  }
}
