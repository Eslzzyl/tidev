import { useState } from "react";
import { useAuthStore } from "../../stores/useAuthStore";

export function SecuritySection() {
  const { token, configureToken, error, clearError } = useAuthStore();
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [success, setSuccess] = useState(false);

  const hasPassword = !!token;

  const handleSetPassword = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newPassword.trim() || newPassword !== confirmPassword || submitting)
      return;
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
      useAuthStore.setState({ error: "Current password is incorrect" });
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
      useAuthStore.setState({ error: "Current password is incorrect" });
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
        Security
      </h2>
      <p className="mb-4 text-base text-neutral-500 dark:text-neutral-400">
        Set a password to protect the web interface
      </p>

      <div className="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
        {hasPassword ? (
          <form onSubmit={handleChangePassword} className="space-y-3">
            <div>
              <label className="mb-1 block text-xs font-medium text-neutral-600 dark:text-neutral-400">
                Current Password
              </label>
              <input
                type="password"
                value={currentPassword}
                onChange={(e) => setCurrentPassword(e.target.value)}
                className="w-full rounded-md border border-neutral-300 bg-white px-3 py-1.5 text-base text-neutral-900 placeholder-neutral-400 outline-none focus:border-neutral-500 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:placeholder-neutral-500"
                placeholder="Enter current password"
              />
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium text-neutral-600 dark:text-neutral-400">
                New Password
              </label>
              <input
                type="password"
                value={newPassword}
                onChange={(e) => setNewPassword(e.target.value)}
                className="w-full rounded-md border border-neutral-300 bg-white px-3 py-1.5 text-base text-neutral-900 placeholder-neutral-400 outline-none focus:border-neutral-500 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:placeholder-neutral-500"
                placeholder="Enter new password"
              />
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium text-neutral-600 dark:text-neutral-400">
                Confirm New Password
              </label>
              <input
                type="password"
                value={confirmPassword}
                onChange={(e) => setConfirmPassword(e.target.value)}
                className="w-full rounded-md border border-neutral-300 bg-white px-3 py-1.5 text-base text-neutral-900 placeholder-neutral-400 outline-none focus:border-neutral-500 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:placeholder-neutral-500"
                placeholder="Confirm new password"
              />
            </div>

            {error && <p className="text-base text-red-500">{error}</p>}
            {success && (
              <p className="text-base text-green-500">
                Password updated successfully
              </p>
            )}

            <div className="flex gap-2">
              <button
                type="submit"
                disabled={
                  !currentPassword.trim() ||
                  !newPassword.trim() ||
                  newPassword !== confirmPassword ||
                  submitting
                }
                className="rounded-md bg-neutral-900 px-3 py-1.5 text-xs font-medium text-white hover:bg-neutral-800 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
              >
                {submitting ? "Saving..." : "Change Password"}
              </button>
              <button
                type="button"
                onClick={handleRemovePassword}
                disabled={!currentPassword.trim() || submitting}
                className="rounded-md border border-red-300 px-3 py-1.5 text-xs font-medium text-red-600 hover:bg-red-50 disabled:cursor-not-allowed disabled:opacity-50 dark:border-red-800 dark:text-red-400 dark:hover:bg-red-950"
              >
                Remove Password
              </button>
            </div>
          </form>
        ) : (
          <form onSubmit={handleSetPassword} className="space-y-3">
            <div>
              <label className="mb-1 block text-xs font-medium text-neutral-600 dark:text-neutral-400">
                New Password
              </label>
              <input
                type="password"
                value={newPassword}
                onChange={(e) => setNewPassword(e.target.value)}
                className="w-full rounded-md border border-neutral-300 bg-white px-3 py-1.5 text-base text-neutral-900 placeholder-neutral-400 outline-none focus:border-neutral-500 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:placeholder-neutral-500"
                placeholder="Enter password"
              />
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium text-neutral-600 dark:text-neutral-400">
                Confirm Password
              </label>
              <input
                type="password"
                value={confirmPassword}
                onChange={(e) => setConfirmPassword(e.target.value)}
                className="w-full rounded-md border border-neutral-300 bg-white px-3 py-1.5 text-base text-neutral-900 placeholder-neutral-400 outline-none focus:border-neutral-500 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:placeholder-neutral-500"
                placeholder="Confirm password"
              />
            </div>

            {error && <p className="text-base text-red-500">{error}</p>}
            {success && (
              <p className="text-base text-green-500">
                Password set successfully
              </p>
            )}

            <button
              type="submit"
              disabled={
                !newPassword.trim() ||
                newPassword !== confirmPassword ||
                submitting
              }
              className="rounded-md bg-neutral-900 px-3 py-1.5 text-xs font-medium text-white hover:bg-neutral-800 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
            >
              {submitting ? "Saving..." : "Set Password"}
            </button>
          </form>
        )}
      </div>
    </section>
  );
}
