import { describe, expect, it } from "vitest";

import type { Model } from "../types/api";
import { filterModelProviderGroups, groupModelsByProvider } from "./modelPicker";

function model(overrides: Partial<Model> = {}): Model {
  return {
    provider_id: "openai",
    provider_display_name: "OpenAI",
    model_id: "gpt-5",
    model_display_name: "GPT-5",
    context_window: 128000,
    connected: true,
    active: false,
    supports_vision: true,
    is_gpt: true,
    thinking_levels: [],
    thinking_level: "",
    ...overrides,
  };
}

describe("model picker helpers", () => {
  it("groups models by provider and puts connected providers first", () => {
    const providers = groupModelsByProvider([
      model({ provider_id: "local", provider_display_name: "Local", connected: false }),
      model(),
      model({ model_id: "gpt-5-mini", model_display_name: "GPT-5 Mini" }),
      model({ provider_id: "anthropic", provider_display_name: "Anthropic", model_id: "claude" }),
    ]);

    expect(providers.map((provider) => provider.id)).toEqual(["openai", "anthropic", "local"]);
    expect(providers[0]?.models.map((item) => item.model_id)).toEqual(["gpt-5", "gpt-5-mini"]);
  });

  it("filters by model identifiers and display names", () => {
    const providers = groupModelsByProvider([
      model(),
      model({ model_id: "gpt-5-mini", model_display_name: "GPT-5 Mini" }),
      model({ provider_id: "anthropic", provider_display_name: "Anthropic", model_id: "claude" }),
    ]);

    expect(filterModelProviderGroups(providers, " mini ")[0]?.models).toHaveLength(1);
    expect(filterModelProviderGroups(providers, "claude").map((item) => item.id)).toEqual([
      "anthropic",
    ]);
  });

  it("returns every model from a provider when the provider matches", () => {
    const providers = groupModelsByProvider([
      model(),
      model({ model_id: "gpt-5-mini", model_display_name: "GPT-5 Mini" }),
      model({ provider_id: "anthropic", provider_display_name: "Anthropic", model_id: "claude" }),
    ]);

    expect(filterModelProviderGroups(providers, "OPENAI")[0]?.models).toHaveLength(2);
  });

  it("preserves the original groups for an empty query and removes empty matches", () => {
    const providers = groupModelsByProvider([model()]);

    expect(filterModelProviderGroups(providers, "   ")).toBe(providers);
    expect(filterModelProviderGroups(providers, "missing")).toEqual([]);
  });
});
