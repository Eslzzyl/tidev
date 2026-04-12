# Prompt Presets

TiDev now ships with a small preset catalog so the model role can be selected by name instead of embedding long prompt strings directly in config.

## Built-in presets

- `tidev_default`: the normal coding assistant prompt. Use this for day-to-day chat.
- `plan`: a short planning prompt that asks for concrete steps, risks, and assumptions before implementation.
- `review`: a review prompt that prioritizes bugs, regressions, and missing tests.
- `apply_patch`: an implementation prompt that favors the smallest safe change and preserves existing style.
- `compact`: a context-summary prompt for continuation after the conversation grows large.
- `provider_setup`: a prompt tuned for provider onboarding and endpoint validation.

## How to use

Set `system_prompt_preset` on a model in `config.toml`:

```toml
[providers.openai.models.gpt-4o-mini]
system_prompt_preset = "tidev_default"
```

If `system_prompt` is also present, it wins and the preset is ignored. Blank custom prompts fall back to the preset or the default TiDev prompt.

## Design notes

The preset catalog lives in `src/prompts.rs` so the same prompt text can be reused by the model config, context compaction, and future agent modes without duplicating strings.
