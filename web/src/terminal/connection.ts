import type { PtyResizeMeta } from "restty";

/** Lifecycle state of a terminal WebSocket. */
export type ConnectionStatus = "disconnected" | "connecting" | "connected";

type DataHandler = (data: string) => void;
type StatusHandler = (status: ConnectionStatus) => void;

/**
 * A reconnecting WebSocket connection for one server-side PTY session.
 *
 * The wire format follows restty's native WebSocket PTY transport. The
 * renderer-specific behavior lives in resttyTransport.ts.
 */
export class TerminalConnection {
  private _ws: WebSocket | null = null;
  private readonly _sessionId: string;
  private _status: ConnectionStatus = "disconnected";
  private readonly _dataHandlers = new Set<DataHandler>();
  private readonly _statusHandlers = new Set<StatusHandler>();
  private _reconnectAttempt = 0;
  private readonly _reconnectLimit = 7;
  private _reconnectTimeout: ReturnType<typeof setTimeout> | null = null;
  private _decoder = new TextDecoder();
  private _disposed = false;

  constructor(sessionId: string) {
    this._sessionId = sessionId;
  }

  get sessionId(): string {
    return this._sessionId;
  }

  get status(): ConnectionStatus {
    return this._status;
  }

  onData(handler: DataHandler): void {
    this._dataHandlers.add(handler);
  }

  offData(handler: DataHandler): void {
    this._dataHandlers.delete(handler);
  }

  onStatusChange(handler: StatusHandler): void {
    this._statusHandlers.add(handler);
  }

  offStatusChange(handler: StatusHandler): void {
    this._statusHandlers.delete(handler);
  }

  connect(): void {
    if (this._disposed) return;
    if (
      this._ws &&
      (this._ws.readyState === WebSocket.OPEN || this._ws.readyState === WebSocket.CONNECTING)
    ) {
      return;
    }

    this._cancelReconnect();
    this._updateStatus("connecting");

    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const params = new URLSearchParams({ session_id: this._sessionId });
    const token = localStorage.getItem("web_auth_token");
    if (token) params.set("token", token);
    const url = `${protocol}//${window.location.host}/api/terminal/ws?${params.toString()}`;

    try {
      const ws = new WebSocket(url);
      ws.binaryType = "arraybuffer";
      this._ws = ws;
      this._decoder = new TextDecoder();

      ws.onopen = () => {
        if (this._ws !== ws) return;
        this._reconnectAttempt = 0;
        this._updateStatus("connected");
      };

      ws.onmessage = (event: MessageEvent) => {
        if (this._ws !== ws) return;

        if (event.data instanceof ArrayBuffer) {
          this._emitData(this._decoder.decode(new Uint8Array(event.data), { stream: true }));
          return;
        }

        if (event.data instanceof Blob) {
          void event.data.arrayBuffer().then((data) => {
            if (this._ws !== ws) return;
            this._emitData(this._decoder.decode(new Uint8Array(data), { stream: true }));
          });
          return;
        }

        if (typeof event.data === "string") {
          this._handleTextMessage(event.data);
        }
      };

      ws.onclose = (event: CloseEvent) => {
        if (this._ws !== ws) return;
        const tail = this._decoder.decode();
        if (tail) this._emitData(tail);

        this._ws = null;
        if (event.code === 1000 || event.code === 1001) {
          this._updateStatus("disconnected");
          return;
        }
        this._scheduleReconnect();
      };

      ws.onerror = () => {
        // The close event owns reconnect handling.
      };
    } catch {
      this._scheduleReconnect();
    }
  }

  sendInput(data: string): boolean {
    return this._send({ type: "input", data });
  }

  resize(cols: number, rows: number, meta?: PtyResizeMeta): boolean {
    return this._send({ type: "resize", cols, rows, ...meta });
  }

  /** Close the current socket while keeping this connection reusable. */
  disconnect(): void {
    this._cancelReconnect();
    this._cleanupSocket();
    this._updateStatus("disconnected");
  }

  /** Permanently release the connection and all reconnect state. */
  dispose(): void {
    this._disposed = true;
    this.disconnect();
    this._dataHandlers.clear();
    this._statusHandlers.clear();
  }

  private _handleTextMessage(data: string): void {
    try {
      const message = JSON.parse(data) as { type?: unknown };
      if (message && message.type === "exit") {
        this._cleanupSocket();
        this._updateStatus("disconnected");
        return;
      }
    } catch {
      // Plain text is terminal output.
    }

    this._emitData(data);
  }

  private _send(message: object): boolean {
    if (!this._ws || this._ws.readyState !== WebSocket.OPEN) return false;
    this._ws.send(JSON.stringify(message));
    return true;
  }

  private _cleanupSocket(): void {
    if (!this._ws) return;
    const ws = this._ws;
    this._ws = null;
    ws.onopen = null;
    ws.onmessage = null;
    ws.onclose = null;
    ws.onerror = null;
    ws.close();
  }

  private _scheduleReconnect(): void {
    if (this._disposed) {
      this._updateStatus("disconnected");
      return;
    }
    if (this._reconnectAttempt >= this._reconnectLimit) {
      this._updateStatus("disconnected");
      return;
    }

    const delay = Math.floor(Math.random() * (1 << this._reconnectAttempt) * 1000);
    this._reconnectAttempt++;
    this._cleanupSocket();
    this._updateStatus("connecting");

    this._reconnectTimeout = setTimeout(() => {
      this._reconnectTimeout = null;
      this.connect();
    }, delay);
  }

  private _cancelReconnect(): void {
    if (this._reconnectTimeout === null) return;
    clearTimeout(this._reconnectTimeout);
    this._reconnectTimeout = null;
  }

  private _updateStatus(status: ConnectionStatus): void {
    if (this._status === status) return;
    this._status = status;
    for (const handler of this._statusHandlers) handler(status);
  }

  private _emitData(data: string): void {
    if (!data) return;
    for (const handler of this._dataHandlers) handler(data);
  }
}
