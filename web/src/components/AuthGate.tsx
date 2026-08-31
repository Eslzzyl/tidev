import { useState, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useAuthStore } from "../stores/useAuthStore";
import { Button, Input } from "./ui";

export function AuthGate() {
  const { t } = useTranslation();
  const [password, setPassword] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const { verifyToken, setToken, error, clearError, isAuthRequired, isAuthenticated } =
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
    } else {
      useAuthStore.setState({
        error: t("Invalid access token. Please try again."),
      });
    }
    setSubmitting(false);
  };

  // If auth is no longer required, or already authenticated, don't render
  if (!isAuthRequired || isAuthenticated) return null;

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
          <h1 className="text-2xl font-bold text-neutral-900 dark:text-neutral-100">tidev</h1>
          <p className="mt-1 text-sm text-neutral-500 dark:text-neutral-400">
            {t("Enter your access token to continue")}
          </p>
        </div>

        {/* Password form */}
        <form onSubmit={handleSubmit}>
          <div className="mb-4">
            <Input
              ref={inputRef}
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder={t("Access token")}
              autoComplete="current-password"
              disabled={submitting}
            />
          </div>

          {error && <p className="mb-4 text-center text-sm text-red-500">{error}</p>}

          <Button
            type="submit"
            disabled={!password.trim() || submitting}
            className="w-full"
            variant="primary"
            size="md"
          >
            {submitting ? t("Verifying...") : t("Unlock")}
          </Button>
        </form>
      </div>
    </div>
  );
}
