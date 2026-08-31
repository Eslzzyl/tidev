import { useState } from "react";
import { ShieldCheck, ShieldAlert } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useAuthStore } from "../../stores/useAuthStore";
import { Button, Input } from "../ui";

export function SecuritySection() {
  const { t } = useTranslation();
  const { token, configureToken, error, clearError } = useAuthStore();
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [success, setSuccess] = useState(false);

  const hasPassword = Boolean(token);

  const handleSetPassword = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newPassword.trim() || newPassword !== confirmPassword || submitting) return;
    clearError();
    setSuccess(false);
    setSubmitting(true);
    const ok = await configureToken(newPassword.trim());
    setSubmitting(false);
    if (ok) {
      setNewPassword("");
      setConfirmPassword("");
      setCurrentPassword("");
      setSuccess(true);
      setTimeout(() => setSuccess(false), 2000);
    }
  };

  const handleChangePassword = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!currentPassword.trim() || !newPassword.trim() || submitting) return;
    clearError();
    setSuccess(false);

    const { verifyToken } = useAuthStore.getState();
    const valid = await verifyToken(currentPassword.trim());
    if (!valid) {
      useAuthStore.setState({ error: t("Current password is incorrect") });
      return;
    }

    setSubmitting(true);
    const ok = await configureToken(newPassword.trim());
    setSubmitting(false);
    if (ok) {
      setNewPassword("");
      setConfirmPassword("");
      setCurrentPassword("");
      setSuccess(true);
      setTimeout(() => setSuccess(false), 2000);
    }
  };

  const handleRemovePassword = async () => {
    if (!currentPassword.trim() || submitting) return;
    clearError();
    setSuccess(false);

    const { verifyToken } = useAuthStore.getState();
    const valid = await verifyToken(currentPassword.trim());
    if (!valid) {
      useAuthStore.setState({ error: t("Current password is incorrect") });
      return;
    }

    setSubmitting(true);
    const ok = await configureToken("");
    setSubmitting(false);
    if (ok) {
      setCurrentPassword("");
      setSuccess(true);
      setTimeout(() => setSuccess(false), 2000);
    }
  };

  return (
    <section className="space-y-6">
      <div>
        <h2 className="text-base font-semibold text-neutral-900 dark:text-neutral-100">
          {t("Security")}
        </h2>
        <p className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">
          {t("Set a password to protect the web interface")}
        </p>
      </div>

      <div className="space-y-3">
        <label className="text-xs font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
          {t("Authentication")}
        </label>
        <div className="rounded-xl border border-neutral-200/80 bg-neutral-50/50 p-4 space-y-4 dark:border-neutral-800/80 dark:bg-neutral-800/30">
          {/* Status Header */}
          <div className="flex items-center justify-between gap-3 pb-3 border-b border-neutral-200/60 dark:border-neutral-800/60">
            <div className="flex items-center gap-3">
              <div
                className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg ${
                  hasPassword
                    ? "bg-emerald-50 text-emerald-600 dark:bg-emerald-950/50 dark:text-emerald-400"
                    : "bg-amber-50 text-amber-600 dark:bg-amber-950/50 dark:text-amber-400"
                }`}
              >
                {hasPassword ? (
                  <ShieldCheck className="h-4 w-4" />
                ) : (
                  <ShieldAlert className="h-4 w-4" />
                )}
              </div>
              <div>
                <span className="block text-xs font-semibold text-neutral-900 dark:text-neutral-100">
                  {hasPassword ? t("Password protected") : t("No password set")}
                </span>
                <span className="block text-[11px] text-neutral-500 dark:text-neutral-400">
                  {hasPassword
                    ? t("Access requires authentication token")
                    : t("Web interface is currently accessible without credentials")}
                </span>
              </div>
            </div>
            <span
              className={`inline-flex rounded-full px-2 py-0.5 text-[10px] font-semibold ${
                hasPassword
                  ? "bg-emerald-100/70 text-emerald-700 dark:bg-emerald-950/80 dark:text-emerald-300"
                  : "bg-amber-100/70 text-amber-700 dark:bg-amber-950/80 dark:text-amber-300"
              }`}
            >
              {hasPassword ? t("Enabled") : t("Disabled")}
            </span>
          </div>

          {/* Form */}
          {hasPassword ? (
            <form onSubmit={handleChangePassword} className="space-y-3">
              <div>
                <label className="mb-1 block text-xs font-medium text-neutral-700 dark:text-neutral-300">
                  {t("Current Password")}
                </label>
                <Input
                  type="password"
                  value={currentPassword}
                  onChange={(e) => setCurrentPassword(e.target.value)}
                  placeholder={t("Enter current password")}
                />
              </div>
              <div className="grid gap-3 sm:grid-cols-2">
                <div>
                  <label className="mb-1 block text-xs font-medium text-neutral-700 dark:text-neutral-300">
                    {t("New Password")}
                  </label>
                  <Input
                    type="password"
                    value={newPassword}
                    onChange={(e) => setNewPassword(e.target.value)}
                    placeholder={t("Enter new password")}
                  />
                </div>
                <div>
                  <label className="mb-1 block text-xs font-medium text-neutral-700 dark:text-neutral-300">
                    {t("Confirm New Password")}
                  </label>
                  <Input
                    type="password"
                    value={confirmPassword}
                    onChange={(e) => setConfirmPassword(e.target.value)}
                    placeholder={t("Confirm new password")}
                  />
                </div>
              </div>

              {error && <p className="text-xs text-red-500">{error}</p>}
              {success && (
                <p className="text-xs text-emerald-600 dark:text-emerald-400">
                  {t("Password updated successfully")}
                </p>
              )}

              <div className="flex items-center gap-2 pt-1">
                <Button
                  type="submit"
                  disabled={
                    !currentPassword.trim() ||
                    !newPassword.trim() ||
                    newPassword !== confirmPassword ||
                    submitting
                  }
                  variant="primary"
                  size="sm"
                  loading={submitting}
                >
                  {t("Change Password")}
                </Button>
                <Button
                  type="button"
                  onClick={handleRemovePassword}
                  disabled={!currentPassword.trim() || submitting}
                  variant="danger"
                  size="sm"
                >
                  {t("Remove Password")}
                </Button>
              </div>
            </form>
          ) : (
            <form onSubmit={handleSetPassword} className="space-y-3">
              <div className="grid gap-3 sm:grid-cols-2">
                <div>
                  <label className="mb-1 block text-xs font-medium text-neutral-700 dark:text-neutral-300">
                    {t("New Password")}
                  </label>
                  <Input
                    type="password"
                    value={newPassword}
                    onChange={(e) => setNewPassword(e.target.value)}
                    placeholder={t("Enter password")}
                  />
                </div>
                <div>
                  <label className="mb-1 block text-xs font-medium text-neutral-700 dark:text-neutral-300">
                    {t("Confirm Password")}
                  </label>
                  <Input
                    type="password"
                    value={confirmPassword}
                    onChange={(e) => setConfirmPassword(e.target.value)}
                    placeholder={t("Confirm password")}
                  />
                </div>
              </div>

              {error && <p className="text-xs text-red-500">{error}</p>}
              {success && (
                <p className="text-xs text-emerald-600 dark:text-emerald-400">
                  {t("Password set successfully")}
                </p>
              )}

              <div className="pt-1">
                <Button
                  type="submit"
                  disabled={!newPassword.trim() || newPassword !== confirmPassword || submitting}
                  variant="primary"
                  size="sm"
                  loading={submitting}
                >
                  {t("Set Password")}
                </Button>
              </div>
            </form>
          )}
        </div>
      </div>
    </section>
  );
}
