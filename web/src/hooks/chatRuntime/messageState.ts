import type { MessageRecord } from "../../types/api";

export function mergeMessageRecords(
  loaded: readonly MessageRecord[],
  pending: readonly MessageRecord[],
): MessageRecord[] {
  const seen = new Set(loaded.map((record) => record.message.id));
  return [
    ...loaded,
    ...pending.filter((record) => {
      if (seen.has(record.message.id)) return false;
      seen.add(record.message.id);
      return true;
    }),
  ];
}
