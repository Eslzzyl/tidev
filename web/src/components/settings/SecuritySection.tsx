import { useState } from "react";
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

  const hasPassword = !!token;

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
    <section>
      <h2 className="mb-1 text-base font-medium text-neutral-900 dark:text-neutral-100">
        {t("Security")}
      </h2>
      <p className="mb-4 text-base text-neutral-500 dark:text-neutral-400">
        {t("Set a password to protect the web interface")}
      </p>

      <div className="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
        {hasPassword ? (
          <form onSubmit={handleChangePassword} className="space-y-3">
            <div>
              <label className="mb-1 block text-xs font-medium text-neutral-600 dark:text-neutral-400">
                {t("Current Password")}
              </label>
              <Input
                type="password"
                value={currentPassword}
                onChange={(e) => setCurrentPassword(e.target.value)}
                placeholder={t("Enter current password")}
              />
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium text-neutral-600 dark:text-neutral-400">
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
              <label className="mb-1 block text-xs font-medium text-neutral-600 dark:text-neutral-400">
                {t("Confirm New Password")}
              </label>
              <Input
                type="password"
                value={confirmPassword}
                onChange={(e) => setConfirmPassword(e.target.value)}
                placeholder={t("Confirm new password")}
              />
            </div>

            {error && <p className="text-base text-red-500">{error}</p>}
            {success && (
              <p className="text-base text-green-500">{t("Password updated successfully")}</p>
            )}

            <div className="flex gap-2">
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
              >
                {submitting ? t("Saving...") : t("Change Password")}
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
            <div>
              <label className="mb-1 block text-xs font-medium text-neutral-600 dark:text-neutral-400">
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
              <label className="mb-1 block text-xs font-medium text-neutral-600 dark:text-neutral-400">
                {t("Confirm Password")}
              </label>
              <Input
                type="password"
                value={confirmPassword}
                onChange={(e) => setConfirmPassword(e.target.value)}
                placeholder={t("Confirm password")}
              />
            </div>

            {error && <p className="text-base text-red-500">{error}</p>}
            {success && (
              <p className="text-base text-green-500">{t("Password set successfully")}</p>
            )}

            <Button
              type="submit"
              disabled={!newPassword.trim() || newPassword !== confirmPassword || submitting}
              variant="primary"
              size="sm"
            >
              {submitting ? t("Saving...") : t("Set Password")}
            </Button>
          </form>
        )}
      </div>
    </section>
  );
}
