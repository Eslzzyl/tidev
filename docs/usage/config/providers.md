# Provider and Model Configuration

tidev supports multiple LLM providers and can switch between them at runtime.
Providers are configured under the `[providers]` key in `config.toml`. Bundled
presets ship with the binary and are available automatically without
configuration. User-defined providers extend or override the bundled ones.

## Provider configuration

Each provider is a key-value entry under `[providers]`. The key becomes the
provider identifier used when selecting the provider at runtime.

```
[providers.my-provider]
display_name = "My Custom Provider"
base_url = "https://api.example.com/v1"
api_type = "openai_chat_completions"
```

| Key | Required | Description |
|-----|----------|-------------|
| `display_name` | Yes for new providers | Human-readable name shown in the UI. When overriding a bundled provider, omitted values inherit from the preset |
| `base_url` | Yes for new providers | Base URL of the API endpoint. When overriding a bundled provider, omitted values inherit from the preset |
| `api_type` | No | Default API protocol for all models under this provider (see below). Can be overridden per-model |

### api_type values

| Value | Description |
|-------|-------------|
| `"openai_chat_completions"` | OpenAI Chat Completions API. This is the default and is compatible with many OpenAI-compatible providers |
| `"anthropic"` | Anthropic Messages API |
| `"openai_responses"` | OpenAI Responses API |
| `"google_gemini"` | Google Gemini API |

### base_url rules per api_type

Each `api_type` appends a specific path suffix to `base_url` to form the final
API endpoint. The table below shows the suffix appended and the **standard
convention** for `base_url`:

| api_type | Appended suffix | Standard base_url convention | Full URL example |
|----------|----------------|------------------------------|-----------------|
| `openai_chat_completions` | `/chat/completions` | `https://api.example.com/v1` — already includes `/v1` | `https://api.example.com/v1/chat/completions` |
| `anthropic` | `/v1/messages` | `https://api.example.com` — does **not** include `/v1` | `https://api.example.com/v1/messages` |
| `openai_responses` | `/v1/responses` | `https://api.example.com` — does **not** include `/v1` | `https://api.example.com/v1/responses` |
| `google_gemini` | `/models/{model_id}:generateContent` | `https://generativelanguage.googleapis.com` | `https://generativelanguage.googleapis.com/models/gemini-pro:generateContent` |

**Why the difference?** The OpenAI Chat Completions convention places `/v1` in
`base_url` and appends only `/chat/completions`. The Anthropic and OpenAI
Responses conventions place the entire path on the appended suffix. tidev
follows these respective upstream conventions.

**Either pattern works.** Thanks to idempotent endpoint construction, you can
use either the standard `base_url` (letting tidev append the suffix) or a full
URL (already containing the suffix) — tidev detects the suffix and avoids
double-pathing:

```toml
# Both of these work identically for anthropic:

# Convention: bare base URL → tidev appends /v1/messages
[providers.my-provider.models.claude]
api_type = "anthropic"
base_url = "https://api.anthropic.com"               # → https://api.anthropic.com/v1/messages

# Full URL: tidev detects /v1/messages is already present, uses as-is
[providers.my-provider.models.claude-alt]
api_type = "anthropic"
base_url = "https://opencode.ai/zen/go/v1/messages"  # → https://opencode.ai/zen/go/v1/messages
```

For `openai_chat_completions`, the same idempotency applies:

```toml
# Convention: base_url includes /v1 → tidev appends /chat/completions
[providers.my-provider.models.gpt]
api_type = "openai_chat_completions"
base_url = "https://api.openai.com/v1"                # → https://api.openai.com/v1/chat/completions

# Full URL: tidev detects /chat/completions is already present, uses as-is
[providers.my-provider.models.gpt-alt]
api_type = "openai_chat_completions"
base_url = "https://api.openai.com/v1/chat/completions"  # → https://api.openai.com/v1/chat/completions
```

**Recommendation:** Use the standard convention for clarity, but when a
provider gives you a full endpoint URL, you can use it directly without
modification.

### Per-model api_type override

When a provider serves models with different API protocols (e.g., an
aggregator that exposes both OpenAI and Anthropic models), set `api_type`
on individual models instead of (or in addition to) the provider level.
The resolution order is:

1. **Model-level** `api_type` — highest priority
2. **Provider-level** `api_type` — fallback
3. **Default** — `openai_chat_completions`

```toml
# A provider that serves mixed API protocols
[providers.my-aggregator]
display_name = "My Aggregator"
base_url = "https://api.example.com"
# No provider-level api_type — each model specifies its own

[providers.my-aggregator.models.gpt-4o]
api_type = "openai_chat_completions"
display_name = "GPT-4o"
context_window = 128000
max_output_tokens = 16384

[providers.my-aggregator.models.claude-sonnet]
api_type = "anthropic"
display_name = "Claude Sonnet"
context_window = 200000
max_output_tokens = 64000
```

### Per-model base_url override

Different API protocols often use different endpoints. When a provider
serves models that need different base URLs, set `base_url` on individual
models. The resolution order is:

1. **Model-level** `base_url` — highest priority
2. **Provider-level** `base_url` — fallback

```toml
[providers.my-aggregator]
display_name = "My Aggregator"
base_url = "https://api.openai.com/v1"

[providers.my-aggregator.models.gpt-4o]
# Inherits base_url from provider — no override needed
api_type = "openai_chat_completions"
display_name = "GPT-4o"
context_window = 128000
max_output_tokens = 16384

[providers.my-aggregator.models.claude-sonnet]
base_url = "https://api.anthropic.com"
api_type = "anthropic"
display_name = "Claude Sonnet"
context_window = 200000
max_output_tokens = 64000

[providers.my-aggregator.models.gemini-pro]
base_url = "https://generativelanguage.googleapis.com"
api_type = "google_gemini"
display_name = "Gemini Pro"
context_window = 1048576
max_output_tokens = 8192
```

## Model configuration

Each provider can have multiple models defined under `[providers.<id>.models]`.

```
[providers.my-provider.models.my-model-key]
request_model_id = "gpt-4o"
display_name = "GPT-4o"
context_window = 128000
max_output_tokens = 16384
temperature = 0.7
supports_streaming = true
supports_images = true
supports_parallel_tool_calls = true
system_prompt = "You are a helpful assistant."
extra_body = { some_provider_param = "value" }
```

| Key | Required | Default | Description |
|-----|----------|---------|-------------|
| `request_model_id` | No | Uses the model key | The model identifier sent to the API. Useful when the API expects a different name than the configuration key |
| `display_name` | Yes | | Human-readable name for the model shown in the UI |
| `context_window` | Yes | | Maximum number of input tokens the model can accept |
| `max_output_tokens` | Yes | | Maximum number of tokens the model can generate in a single response |
| `api_type` | No | Inherited from provider | Per-model API protocol override. See [api_type values](#api_type-values) above. When omitted, uses the provider-level `api_type` |
| `temperature` | Yes | | Sampling temperature for the model. Higher values produce more random outputs |
| `supports_streaming` | No | `true` | Whether the model supports streaming responses |
| `supports_images` | No | `false` | Whether the model can process image inputs |
| `supports_parallel_tool_calls` | No | `true` | Whether the model can return multiple tool calls in one response |
| `system_prompt` | No | None | Custom system prompt override. When set, this replaces the default system prompt for this model |
| `extra_body` | No | None | Additional JSON fields to include in the API request body. Used for provider-specific parameters |

## API key management

API keys are stored in `~/.local/share/tidev/auth.json` and are managed
separately from the config file. The auth file has three sections:

- `providers` -- API keys for LLM providers
- `channels` -- Credentials for gateway channels such as Telegram and QQ
- `web` -- Web UI authentication token

```
{
  "providers": {
    "openai": {
      "api_key": "sk-..."
    },
    "anthropic": {
      "api_key": "sk-ant-..."
    }
  },
  "channels": {
    "telegram": {
      "api_key": "bot-token-here"
    },
    "qq": {
      "api_key": "app-id-here",
      "extra": {
        "app_secret": "app-secret-here"
      }
    }
  },
  "web": {
    "auth_token": "optional-bearer-token"
  }
}
```

Each channel entry has an `api_key` field and an optional `extra` map for
additional deployment-specific credentials. For example, the QQ channel uses
`extra.app_secret` for its application secret.

You can set API keys through the TUI or by editing the file directly. When the
TUI starts, only providers with a configured API key appear as available
options. Providers without a key are hidden from the model selection.

## Bundled presets

tidev ships with bundled provider presets that are merged into the available
providers at startup. These presets live in `presets.toml` at the repository root
and are compiled into the binary. Users do not need to copy these into their
config file, but can override them by defining a provider with the same key in
their user config.

Each bundled provider includes multiple model configurations with their
respective context windows, output limits, and temperature settings.

## Model resolution

When a session starts, tidev resolves the active model from the configured
defaults. The resolution follows this order:

1. If a query string of the form `"provider/model_id"` is given, that provider
   and model are used.
2. If only a model ID is given, it is resolved against the current provider.
3. If no query is given, `default_provider` and `default_model` are used.

A model ID can be ambiguous if multiple providers define a model with the same
key. In that case, tidev lists the available choices and requires disambiguation.

### Gateway model resolution

In gateway mode, a separate model resolution path is used. If
`gateway.default_provider` or `gateway.default_model` is set, those values take
precedence over the global defaults. The gateway model also uses a different
system prompt (`gateway_system_prompt`) tailored for messaging-platform
interaction.

### Sub-agent model resolution

When a sub-agent type has a configured model in `[agent.models]`, it is used
instead of the parent session's model. The value can be a plain model ID
(using the parent session's provider) or in `"provider/model_id"` format to
specify both. If no model is configured for an agent type, the sub-agent
inherits the parent session's active model.

## Provider override example

To add a custom model to a bundled provider, define only the model entry under
the same provider key:

```
[providers.deepseek.models.my-custom-model]
request_model_id = "deepseek-v4-custom"
display_name = "DeepSeek V4 Custom"
context_window = 1048576
max_output_tokens = 262144
temperature = 0.7
```

To override a bundled provider's base URL, define the provider key with a new
base URL:

```
[providers.deepseek]
display_name = "My DeepSeek Mirror"
base_url = "https://my-mirror.example.com"
```

Model entries in the user config are merged with bundled ones. If a user model
has the same key as a bundled model, the user model replaces it entirely.
Provider-level `display_name`, `base_url`, and `api_type` values follow the same
override rule: values present in the user config replace the bundled values,
while omitted values are inherited. This also applies to project-level
`.tidev/config.toml` overlays.

## Connecting to custom providers

Any OpenAI-compatible API can be used by defining a provider with
`api_type = "openai_chat_completions"` (the default). For example, to use a
local LLM server:

```
[providers.local]
display_name = "Local LLM"
base_url = "http://localhost:8080/v1"

[providers.local.models.llama-3]
request_model_id = "llama-3-8b"
display_name = "Llama 3 8B"
context_window = 8192
max_output_tokens = 2048
temperature = 0.7
supports_streaming = true
supports_images = false
```

## Extra body for provider-specific features

The `extra_body` field on a model config allows passing arbitrary JSON to the
API request body. This is used by certain providers for features like thinking
mode, reasoning effort, or custom parameters.

For example, to enable thinking on a DeepSeek V4 model through the user config:

```
[providers.deepseek.models.deepseek-v4-pro]
extra_body = { thinking = { type = "enabled" }, reasoning_effort = "high" }
```

When the thinking level is toggled in the TUI or configured in
`[agent.thinking_levels]`, the extra body from the config and the thinking
configuration are merged together, with the thinking-level-generated fields
taking precedence.

## Embedding models

tidev supports vector search for memory using embedding models. Embedding models
are configured **nested under providers**, using the same pattern as LLM models:

```toml
[providers.openai]
display_name = "OpenAI"
base_url = "https://api.openai.com/v1"
api_type = "openai_chat_completions"

[providers.openai.models.gpt-4o-mini]
display_name = "GPT-4o-mini"
context_window = 128000
max_output_tokens = 16384
temperature = 0.7

[providers.openai.embedding_models.text-embedding-3-small]
model_id = "text-embedding-3-small"
display_name = "Text Embedding 3 Small"
context_window = 8191
dimensions = 1536
```

Embedding models share the provider's `base_url` and `api_key`.

| Key | Required | Description |
|-----|----------|-------------|
| `model_id` | Yes | The model identifier sent to the `/embeddings` API |
| `display_name` | Yes | Human-readable name shown in the UI |
| `context_window` | Yes | Maximum number of input tokens the model can accept |
| `dimensions` | Yes | Output vector dimension (e.g. 1536 for text-embedding-3-small) |

When at least one embedding model is configured and the provider has an API key
available, tidev uses the embedding model for vector search via
`LlmClient::embed()`, which shares the same retry/backoff infrastructure as
LLM completions. If no embedding model is configured, vector search degrades to
FTS5 full-text search.

Multiple embedding models can be configured per provider. The first available one is used by
default; a specific model can be selected in the model panel.

Some popular embedding models are also included in `presets.toml` (bundled with the binary)
under their respective providers — they're available without any extra configuration.

## Memory model overrides

By default, memory operations (compression, summarization) use the same active
model as the chat session. You can override these with separate models via the
`[memory]` section:

```toml
[memory]
compression_model = "openai/gpt-4o-mini"     # Optional override for compression
summarization_model = ""                      # Empty = inherit from compression_model
embedding_model = "openai/text-embedding-3-small"  # Optional override for embedding
```

| Key | Description |
|-----|-------------|
| `compression_model` | Model used for compressing observations. Format: `"provider/model_id"`. Default: inherits from session model |
| `summarization_model` | Model used for session summarization. Falls back to `compression_model`, then to session model |
| `embedding_model` | Model used for generating embeddings. Must match a `[providers.*.embedding_models.*]` entry. Default: first available embedding model |

These overrides can also be configured through the model panel (open with `/model`
or `Ctrl+M`) by navigating to the **Memory** tab and using `←`/`→` to select
between Compression, Summarization, and Embedding sub-entries.
