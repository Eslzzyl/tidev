import type { AppEvent } from '../types/events';

export type EventCallback = (event: AppEvent) => void;

export class SSEClient {
  private eventSource: EventSource | null = null;
  private sessionId: string | null = null;
  private callbacks: Map<string, EventCallback[]> = new Map();
  private reconnectAttempts = 0;
  private maxReconnectAttempts = 5;
  private reconnectDelay = 1000;

  connect(sessionId: string) {
    if (this.eventSource) {
      this.disconnect();
    }

    this.sessionId = sessionId;
    this.reconnectAttempts = 0;
    this.connectInternal();
  }

  private connectInternal() {
    if (!this.sessionId) return;

    const url = `/api/events?session=${this.sessionId}`;
    this.eventSource = new EventSource(url);

    this.eventSource.onopen = () => {
      this.reconnectAttempts = 0;
      this.emit('connected', undefined as unknown as AppEvent);
    };

    this.eventSource.onerror = () => {
      this.emit('error', undefined as unknown as AppEvent);

      if (this.reconnectAttempts < this.maxReconnectAttempts) {
        this.reconnectAttempts++;
        setTimeout(() => {
          this.connectInternal();
        }, this.reconnectDelay * this.reconnectAttempts);
      }
    };

    this.eventSource.addEventListener('message.chunk', (e: MessageEvent) => {
      this.emit('message.chunk', JSON.parse(e.data));
    });

    this.eventSource.addEventListener('reasoning.chunk', (e: MessageEvent) => {
      this.emit('reasoning.chunk', JSON.parse(e.data));
    });

    this.eventSource.addEventListener('message.complete', (e: MessageEvent) => {
      this.emit('message.complete', JSON.parse(e.data));
    });

    this.eventSource.addEventListener('tool.call', (e: MessageEvent) => {
      this.emit('tool.call', JSON.parse(e.data));
    });

    this.eventSource.addEventListener('tool.result', (e: MessageEvent) => {
      this.emit('tool.result', JSON.parse(e.data));
    });

    this.eventSource.addEventListener('permission.request', (e: MessageEvent) => {
      this.emit('permission.request', JSON.parse(e.data));
    });

    this.eventSource.addEventListener('aborted', (e: MessageEvent) => {
      this.emit('aborted', JSON.parse(e.data));
    });

    this.eventSource.addEventListener('error', (e: MessageEvent) => {
      try {
        const data = e.data ? JSON.parse(e.data) : { error: 'Connection error' };
        this.emit('error', data);
      } catch {
        this.emit('error', { error: 'Connection error' } as unknown as AppEvent);
      }
    });

    this.eventSource.addEventListener('heartbeat', () => {
      this.emit('heartbeat', undefined as unknown as AppEvent);
    });
  }

  disconnect() {
    if (this.eventSource) {
      this.eventSource.close();
      this.eventSource = null;
    }
    this.sessionId = null;
  }

  on(event: string, callback: EventCallback) {
    if (!this.callbacks.has(event)) {
      this.callbacks.set(event, []);
    }
    this.callbacks.get(event)!.push(callback);
  }

  off(event: string, callback: EventCallback) {
    const callbacks = this.callbacks.get(event);
    if (callbacks) {
      const index = callbacks.indexOf(callback);
      if (index > -1) {
        callbacks.splice(index, 1);
      }
    }
  }

  private emit(event: string, data: AppEvent) {
    const callbacks = this.callbacks.get(event);
    if (callbacks) {
      callbacks.forEach((cb) => cb(data));
    }
  }

  isConnected(): boolean {
    return this.eventSource !== null && this.eventSource.readyState === EventSource.OPEN;
  }
}

export const sseClient = new SSEClient();
