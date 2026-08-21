import type { EventEnvelope } from "../types/api";

export function eventPayload(envelope: EventEnvelope): [string, Record<string, unknown>] {
  const entry = Object.entries(envelope.event)[0];
  return entry ? [entry[0], (entry[1] ?? {}) as Record<string, unknown>] : ["", {}];
}

export function asString(value: unknown): string {
  return typeof value === "string" ? value : "";
}
