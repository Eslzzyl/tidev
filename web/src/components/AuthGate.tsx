import { useState, useEffect, useRef } from "react";
import { useAuthStore } from "../stores/useAuthStore";

export function AuthGate() {
  const [password, setPassword] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const { verifyToken, setToken, error, clearError, isAuthRequired } =
    useAuthStore();
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!password.trim() || submitting) return;

    setSubmitting(true);
    clearError();

    const valid = await verifyToken(password.trim());
    if (valid) {
      setToken(password.trim());
    }
    setSubmitting(false);
  };

  // If auth is no longer required, don't render
  if (!isAuthRequired) return null;

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-neutral-50 dark:bg-neutral-950">
      <div className="w-full max-w-sm px-6">
        {/* Logo & title */}
        <div className="mb-8 text-center">
          <div className="mx-auto mb-4 flex h-16 w-16 items-center justify-center rounded-2xl bg-neutral-900 dark:bg-neutral-800">
            <svg
              className="h-8 w-8 text-white"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M12 2L2 7l10 5 10-5-10-5z" />
              <path d="M2 17l10 5 10-5" />
              <path d="M2 12l10 5 10-5" />
            </svg>
          </div>
          <h1 className="text-2xl font-bold text-neutral-900 dark:text-neutral-100">
            TiDev
          </h1>
          <p className="mt-1 text-sm text-neutral-500 dark:text-neutral-400">
            Enter your access token to continue
          </p>
        </div>

        {/* Password form */}
        <form onSubmit={handleSubmit}>
          <div className="mb-4">
            <input
              ref={inputRef}
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="Access token"
              autoComplete="current-password"
              className="w-full rounded-lg border border-neutral-300 bg-white px-4 py-2.5 text-sm text-neutral-900 placeholder-neutral-400 outline-none transition-colors focus:border-neutral-500 focus:ring-1 focus:ring-neutral-500 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:placeholder-neutral-500 dark:focus:border-neutral-500"
              disabled={submitting}
            />
          </div>

          {error && (
            <p className="mb-4 text-center text-sm text-red-500">{error}</p>
          )}

          <button
            type="submit"
            disabled={!password.trim() || submitting}
            className="w-full rounded-lg bg-neutral-900 px-4 py-2.5 text-sm font-medium text-white transition-colors hover:bg-neutral-800 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
          >
            {submitting ? "Verifying..." : "Unlock"}
          </button>
        </form>
      </div>
    </div>
  );
}
