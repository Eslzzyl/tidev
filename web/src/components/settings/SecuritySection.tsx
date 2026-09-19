import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useAuthStore } from "../../stores/useAuthStore";
import { Button, Input } from "../ui";
import { SettingsSectionHeader, SettingsGroup, SettingsRow } from "./SettingsCommon";

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
      <SettingsSectionHeader
        title={t("Security")}
        description={t("Set a password to protect the web interface")}
      />

      <SettingsGroup title={t("Authentication")}>
        <SettingsRow
          label={hasPassword ? t("Password protected") : t("No password set")}
          description={
            hasPassword
              ? t("Access requires authentication token")
              : t("Web interface is currently accessible without credentials")
          }
          control={
            <span
              className={`inline-flex rounded-full px-2.5 py-0.5 text-[11px] font-medium ${
                hasPassword
                  ? "bg-emerald-100/70 text-emerald-700 dark:bg-emerald-950/80 dark:text-emerald-300"
                  : "bg-amber-100/70 text-amber-700 dark:bg-amber-950/80 dark:text-amber-300"
              }`}
            >
              {hasPassword ? t("Enabled") : t("Disabled")}
            </span>
          }
        />
      </SettingsGroup>

      <SettingsGroup title={hasPassword ? t("Change Password") : t("Set Password")}>
        <div className="p-4 space-y-4">
          {hasPassword ? (
            <form onSubmit={handleChangePassword} className="space-y-3 max-w-md">
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
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
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
            <form onSubmit={handleSetPassword} className="space-y-3 max-w-md">
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
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
      </SettingsGroup>
    </section>
  );
}
