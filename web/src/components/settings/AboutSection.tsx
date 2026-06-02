import { useUIStore } from "../../stores/useUIStore";

function ConnectionStatus() {
  const connectionStatus = useUIStore((s) => s.connectionStatus);

  const statusConfig: Record<string, { color: string; label: string }> = {
    connected: { color: "text-green-600", label: "Connected" },
    disconnected: { color: "text-red-600", label: "Disconnected" },
    connecting: { color: "text-yellow-600", label: "Connecting..." },
  };

  const config = statusConfig[connectionStatus] ?? statusConfig.disconnected;

  return (
    <div className="flex items-center justify-between">
      <span className="text-sm text-neutral-600 dark:text-neutral-400">Server</span>
      <span className={`flex items-center gap-1.5 text-sm font-medium ${config.color}`}>
        <span
          className={`h-2 w-2 rounded-full ${
            connectionStatus === "connected"
              ? "bg-green-500"
              : connectionStatus === "connecting"
                ? "bg-yellow-500"
                : "bg-red-500"
          }`}
        />
        {config.label}
      </span>
    </div>
  );
}

export function AboutSection() {
  return (
    <section>
      <h2 className="mb-1 text-sm font-medium text-neutral-900 dark:text-neutral-100">About</h2>
      <p className="mb-4 text-sm text-neutral-500 dark:text-neutral-400">tidev Web Frontend</p>

      <div className="space-y-3">
        <div className="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
          <div className="flex items-center justify-between">
            <span className="text-sm text-neutral-600 dark:text-neutral-400">Version</span>
            <span className="text-sm text-neutral-900 dark:text-neutral-100">0.1.0</span>
          </div>
        </div>

        <div className="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
          <ConnectionStatus />
        </div>
      </div>
    </section>
  );
}
