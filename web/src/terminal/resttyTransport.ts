import type { PtyCallbacks, PtyResizeMeta, PtyTransport } from "restty";
import type { TerminalConnection } from "./connection";

/** Adapt tidev's reconnecting session connection to restty's native PTY API. */
export function createResttyTransport(connection: TerminalConnection): PtyTransport {
  let callbacks: PtyCallbacks | null = null;

  const handleData = (data: string) => callbacks?.onData?.(data);
  const handleStatus = (status: "disconnected" | "connecting" | "connected") => {
    if (status === "connected") callbacks?.onConnect?.();
    if (status === "disconnected") callbacks?.onDisconnect?.();
  };

  return {
    connect: ({ cols, rows, callbacks: nextCallbacks }) => {
      callbacks = nextCallbacks;
      connection.onData(handleData);
      connection.onStatusChange(handleStatus);

      if (connection.status === "connected") {
        callbacks.onConnect?.();
        if (cols && rows) connection.resize(cols, rows);
      } else {
        connection.connect();
      }
    },
    disconnect: () => {
      connection.offData(handleData);
      connection.offStatusChange(handleStatus);
      callbacks = null;
      connection.disconnect();
    },
    sendInput: (data) => connection.sendInput(data),
    resize: (cols, rows, meta?: PtyResizeMeta) => connection.resize(cols, rows, meta),
    isConnected: () => connection.status === "connected",
    destroy: () => {
      connection.offData(handleData);
      connection.offStatusChange(handleStatus);
      callbacks = null;
      connection.disconnect();
    },
  };
}
