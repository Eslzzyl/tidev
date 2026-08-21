import { getAuthToken } from "../stores/useAuthStore";
import type { EventEnvelope, FrontendRequest } from "../types/api";

function eventUrl(path: string, params: URLSearchParams): string {
  const query = params.toString();
  return `${path}${query ? `?${query}` : ""}`;
}

function addAuthToken(params: URLSearchParams): void {
  const token = getAuthToken();
  if (token) params.set("token", token);
}

export function parseBackendEvent(data: string): EventEnvelope | null {
  try {
    const value = JSON.parse(data) as Partial<EventEnvelope>;
    if (
      typeof value.cursor !== "number" ||
      typeof value.session_id !== "string" ||
      !value.event ||
      typeof value.event !== "object"
    ) {
      return null;
    }
    return value as EventEnvelope;
  } catch {
    return null;
  }
}

export function parseFrontendRequest(data: string): FrontendRequest | null {
  try {
    const value = JSON.parse(data) as Partial<FrontendRequest>;
    if (
      typeof value.request_id !== "string" ||
      typeof value.session_id !== "string" ||
      !value.kind
    ) {
      return null;
    }
    return value as FrontendRequest;
  } catch {
    return null;
  }
}

export function openBackendEvents(
  after: number | null,
  onEvent: (event: EventEnvelope) => void,
  onResync: () => void,
  onError: () => void,
): EventSource {
  const params = new URLSearchParams();
  if (after !== null) params.set("after", String(after));
  addAuthToken(params);

  const source = new EventSource(eventUrl("/api/events", params));
  source.addEventListener("backend_event", (event) => {
    const envelope = parseBackendEvent((event as MessageEvent<string>).data);
    if (envelope) onEvent(envelope);
    else onError();
  });
  source.addEventListener("resync_required", onResync);
  source.onerror = onError;
  return source;
}

export function openFrontendRequests(
  onRequest: (request: FrontendRequest) => void,
  onError: () => void,
): EventSource {
  const params = new URLSearchParams();
  addAuthToken(params);

  const source = new EventSource(eventUrl("/api/requests", params));
  source.addEventListener("frontend_request", (event) => {
    const request = parseFrontendRequest((event as MessageEvent<string>).data);
    if (request) onRequest(request);
    else onError();
  });
  source.onerror = onError;
  return source;
}
