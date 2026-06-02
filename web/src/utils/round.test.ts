import { describe, it, expect } from "vitest";
import { buildRounds } from "./round";
import type { Message } from "../types/api";
import type { Round, SystemMessageBlock } from "../types/round";

function userMsg(overrides: Partial<Message> = {}): Message {
  return {
    id: "u1",
    role: "user",
    content: "hello",
    created_at: "2026-05-22T12:00:00Z",
    ...overrides,
  };
}

function assistantMsg(overrides: Partial<Message> = {}): Message {
  return {
    id: "a1",
    role: "assistant",
    content: "hi there",
    created_at: "2026-05-22T12:00:01Z",
    ...overrides,
  };
}

function toolMsg(overrides: Partial<Message> = {}): Message {
  return {
    id: "t1",
    role: "tool",
    content: "file contents",
    tool_call_id: "tc1",
    created_at: "2026-05-22T12:00:02Z",
    ...overrides,
  };
}

function systemMsg(overrides: Partial<Message> = {}): Message {
  return {
    id: "s1",
    role: "system",
    content: "compaction summary",
    created_at: "2026-05-22T12:00:03Z",
    ...overrides,
  };
}

describe("buildRounds", () => {
  it("returns empty array for empty messages", () => {
    expect(buildRounds([])).toEqual([]);
  });

  it("creates a user_only round from a single user message", () => {
    const msg = userMsg();
    const rounds = buildRounds([msg]);
    expect(rounds).toHaveLength(1);
    const round = rounds[0];
    // @ts-expect-error: discriminated union
    expect(round.userMessage).toBe(msg);
    // @ts-expect-error: discriminated union
    expect(round.status).toBe("user_only");
    // @ts-expect-error: discriminated union
    expect(round.segments).toEqual([]);
  });

  it("groups user + assistant messages into one round", () => {
    const msgs = [userMsg(), assistantMsg()];
    const rounds = buildRounds(msgs);
    expect(rounds).toHaveLength(1);
    const round = rounds[0] as Round;
    expect(round.status).toBe("complete");
    expect(round.segments).toHaveLength(1);
    expect(round.segments[0]).toEqual({ type: "text", content: "hi there" });
  });

  it("groups user + assistant + tool messages into one round", () => {
    const msgs = [
      userMsg(),
      assistantMsg({
        tool_calls: [{ id: "tc1", name: "read", arguments: '{"file":"x"}' }],
      }),
      toolMsg({ content: "file content" }),
    ];
    const rounds = buildRounds(msgs);
    expect(rounds).toHaveLength(1);
    const round = rounds[0] as Round;
    expect(round.status).toBe("complete");
    expect(round.segments).toEqual([
      { type: "text", content: "hi there" },
      { type: "tool_call", toolCallId: "tc1" },
    ]);
    expect(round.toolCallMap["tc1"]?.result?.output).toBe("file content");
  });

  it("sets status to streaming for incomplete assistant messages", () => {
    const msgs = [userMsg(), assistantMsg({ streaming: true })];
    const rounds = buildRounds(msgs);
    expect(rounds).toHaveLength(1);
    expect((rounds[0] as Round).status).toBe("streaming");
  });

  it("appends reasoning segment when assistant has reasoning field", () => {
    const msgs = [userMsg(), assistantMsg({ reasoning: "thinking...", content: "answer" })];
    const rounds = buildRounds(msgs);
    const segments = (rounds[0] as Round).segments;
    expect(segments).toHaveLength(2);
    expect(segments[0]).toEqual({ type: "reasoning", content: "thinking..." });
    expect(segments[1]).toEqual({ type: "text", content: "answer" });
  });

  it("merges consecutive text segments from multiple assistant messages", () => {
    const msgs = [
      userMsg(),
      assistantMsg({ id: "a1", content: "part1" }),
      assistantMsg({ id: "a2", content: "part2" }),
    ];
    const rounds = buildRounds(msgs);
    const segments = (rounds[0] as Round).segments;
    expect(segments).toHaveLength(1);
    expect(segments[0]).toEqual({ type: "text", content: "part1\npart2" });
  });

  it("creates a system block for system messages", () => {
    const msg = systemMsg();
    const rounds = buildRounds([msg]);
    expect(rounds).toHaveLength(1);
    const block = rounds[0] as SystemMessageBlock;
    expect(block.kind).toBe("system");
    expect(block.message).toBe(msg);
  });

  it("places unknown role messages as system blocks", () => {
    const msg = userMsg({ role: "error", content: "something broke" });
    const rounds = buildRounds([msg]);
    expect(rounds).toHaveLength(1);
    const block = rounds[0] as SystemMessageBlock;
    expect(block.kind).toBe("system");
    expect(block.message).toBe(msg);
  });

  it("interleaves system blocks between user rounds", () => {
    const msgs = [
      userMsg({ id: "u1" }),
      assistantMsg({ id: "a1" }),
      systemMsg({ id: "s1", content: "compacting..." }),
      userMsg({ id: "u2", content: "next question" }),
    ];
    const rounds = buildRounds(msgs);
    expect(rounds).toHaveLength(3);
    expect((rounds[0] as Round).userMessage?.id).toBe("u1");
    expect((rounds[1] as SystemMessageBlock).kind).toBe("system");
    expect((rounds[1] as SystemMessageBlock).message.id).toBe("s1");
    expect((rounds[2] as Round).userMessage?.id).toBe("u2");
  });
});
