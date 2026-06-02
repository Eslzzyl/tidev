/**
 * Terminal WebSocket connection with built-in reconnection and message queue.
 *
 * Protocol: JSON arrays over WebSocket text frames.
 *   Client → Server: ["stdin", "<text>"], ["resize", <rows>, <cols>]
 *   Server → Client: ["setup"], ["stdout", "<text>"], ["disconnect", "<reason>"]
 */

export type ConnectionStatus = "disconnected" | "connecting" | "connected";

export interface TerminalMessage {
  type: string;
  content: unknown[];
}

type MessageHandler = (msg: TerminalMessage) => void;
type StatusHandler = (status: ConnectionStatus) => void;

export class TerminalConnection {
  private _ws: WebSocket | null = null;
  private _name: string;
  private _status: ConnectionStatus = "disconnected";
  private _messageHandlers: Set<MessageHandler> = new Set();
  private _statusHandlers: Set<StatusHandler> = new Set();
  private _pendingMessages: string[] = [];
  private _reconnectAttempt = 0;
  private _reconnectLimit = 7;
  private _reconnectTimeout: ReturnType<typeof setTimeout> | null = null;
  private _disposed = false;

  constructor(name: string) {
    this._name = name;
  }

  get name(): string {
    return this._name;
  }

  get status(): ConnectionStatus {
    return this._status;
  }

  get disposed(): boolean {
    return this._disposed;
  }

  onMessage(handler: MessageHandler): void {
    this._messageHandlers.add(handler);
  }

  offMessage(handler: MessageHandler): void {
    this._messageHandlers.delete(handler);
  }

  onStatusChange(handler: StatusHandler): void {
    this._statusHandlers.add(handler);
  }

  offStatusChange(handler: StatusHandler): void {
    this._statusHandlers.delete(handler);
  }

  connect(): void {
    if (this._disposed) return;
    // Guard: don't re-connect if already connecting or connected
    if (this._ws && (this._ws.readyState === WebSocket.OPEN || this._ws.readyState === WebSocket.CONNECTING)) {
      return;
    }
    this._cancelReconnect();
    this._updateStatus("connecting");

    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const token = localStorage.getItem("web_auth_token");
    const tokenParam = token ? `?token=${encodeURIComponent(token)}` : "";
    const url = `${protocol}//${window.location.host}/api/terminal/ws${tokenParam}`;

    try {
      const ws = new WebSocket(url);
      this._ws = ws;

      ws.onopen = () => {
        // Send bind message
        this._sendRaw(JSON.stringify(["bind", this._name]));
      };

      ws.onmessage = (event: MessageEvent) => {
        try {
          const data = JSON.parse(event.data) as unknown[];
          if (!Array.isArray(data) || data.length === 0) return;

          const msgType = String(data[0]);
          const content = data.slice(1);

          // Handle protocol-level messages
          if (msgType === "setup") {
            this._updateStatus("connected");
            this._flushQueue();
            return;
          }

          if (msgType === "disconnect") {
            this._updateStatus("disconnected");
            this._cleanup();
            return;
          }

          // Forward to handlers
          this._emitMessage({ type: msgType, content });
        } catch {
          // Invalid JSON — ignore
        }
      };

      ws.onclose = (event: CloseEvent) => {
        // Normal close (1000) or going away (1001) — no reconnect
        if (event.code === 1000 || event.code === 1001) {
          this._updateStatus("disconnected");
          this._cleanup();
          return;
        }
        // Unexpected close — attempt reconnect
        this._scheduleReconnect();
      };

      ws.onerror = () => {
        // onclose will follow, which triggers reconnect
      };
    } catch {
      // WebSocket constructor failed — attempt reconnect
      this._scheduleReconnect();
    }
  }

  sendMessage(type: string, ...content: unknown[]): void {
    const msg = JSON.stringify([type, ...content]);

    if (this._status === "connected" && this._ws) {
      this._sendRaw(msg);
    } else {
      // Queue for later delivery
      this._pendingMessages.push(msg);
    }
  }

  disconnect(): void {
    this._disposed = true;
    this._cancelReconnect();
    this._cleanup();
    this._updateStatus("disconnected");
  }

  private _sendRaw(msg: string): void {
    if (this._ws && this._ws.readyState === WebSocket.OPEN) {
      this._ws.send(msg);
    }
  }

  private _flushQueue(): void {
    const pending = this._pendingMessages;
    this._pendingMessages = [];
    for (const msg of pending) {
      this._sendRaw(msg);
    }
  }

  private _cleanup(): void {
    if (this._ws) {
      this._ws.onopen = null;
      this._ws.onmessage = null;
      this._ws.onclose = null;
      this._ws.onerror = null;
      this._ws.close();
      this._ws = null;
    }
  }

  private _scheduleReconnect(): void {
    if (this._disposed) return;
    if (this._reconnectAttempt >= this._reconnectLimit) {
      this._updateStatus("disconnected");
      return;
    }

    // Exponential backoff with random jitter
    const delay = Math.floor(Math.random() * (1 << this._reconnectAttempt) * 1000);
    this._reconnectAttempt++;

    this._cleanup();
    this._updateStatus("connecting");

    this._reconnectTimeout = setTimeout(() => {
      this._reconnectTimeout = null;
      this.connect();
    }, delay);
  }

  private _cancelReconnect(): void {
    if (this._reconnectTimeout !== null) {
      clearTimeout(this._reconnectTimeout);
      this._reconnectTimeout = null;
    }
  }

  private _updateStatus(status: ConnectionStatus): void {
    if (this._status !== status) {
      this._status = status;
      for (const handler of this._statusHandlers) {
        handler(status);
      }
    }
  }

  private _emitMessage(msg: TerminalMessage): void {
    for (const handler of this._messageHandlers) {
      handler(msg);
    }
  }
}
