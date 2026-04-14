# Provider and Model Configuration

TiDev uses a TOML configuration file (`config.toml`) to define providers and models.

## Configuration Location

- **User config**: `~/.config/tidev/config.toml`
- **Bundled presets**: `presets.toml` in the repo root (merged at runtime)

## Provider Configuration

A provider represents an LLM service endpoint.

```toml
[providers.<provider-id>]
display_name = "Provider Display Name"
base_url = "https://api.example.com/v1"
api_type = "openai"  # optional: "openai" (default) or "anthropic"
```

### Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `display_name` | string | Yes | Human-readable name shown in UI |
| `base_url` | string | Yes | API endpoint base URL |
| `api_type` | string | No | API format: `openai` (default) or `anthropic` |

## Model Configuration

Each provider can have one or more models.

```toml
[providers.<provider-id>.models.<model-id>]
display_name = "Model Display Name"
context_window = 128000
max_output_tokens = 8192
temperature = 0.7
supports_streaming = true
supports_images = false
system_prompt = "Optional system prompt"  # optional
extra_body = {}  # optional
```

### Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `display_name` | string | Yes | Human-readable name shown in UI |
| `context_window` | integer | Yes | Maximum context window size in tokens |
| `max_output_tokens` | integer | Yes | Maximum output tokens per response |
| `temperature` | float | Yes | Default sampling temperature (0.0 - 2.0) |
| `supports_streaming` | boolean | No | Whether streaming is supported (default: true) |
| `supports_images` | boolean | No | Whether vision/image input is supported (default: false) |
| `system_prompt` | string | No | Default system prompt for this model |
| `extra_body` | table | No | Additional API request parameters |

## Extra Body

The `extra_body` field allows you to pass custom parameters to the API request. These fields are merged into the request body.

### Example: Custom Temperature and Top P

```toml
[providers.openai.models.gpt-4]
display_name = "GPT-4"
context_window = 128000
max_output_tokens = 4096
temperature = 1.0
extra_body = { top_p = 0.9, frequency_penalty = 0.5 }
```

### Example: Reasoning Model

```toml
[providers.anthropic.models.claude-sonnet-4-20250514]
display_name = "Claude Sonnet 4"
context_window = 200000
max_output_tokens = 8192
temperature = 1.0
extra_body = { thinking = { type = "enabled", budget_tokens = 1024 } }
```

### Example: Response Format

```toml
[providers.openai.models.gpt-4o]
display_name = "GPT-4o"
context_window = 128000
max_output_tokens = 4096
temperature = 1.0
extra_body = { response_format = { type = "json_object" } }
```

## Bundled Providers

TiDev ships with a bundled DeepSeek provider:

```toml
# This is pre-configured and available out of the box
[providers.deepseek]
display_name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"

[providers.deepseek.models.deepseek-chat]
display_name = "DeepSeek Chat"
context_window = 128000
max_output_tokens = 8192
temperature = 0.7
```

## Authentication

API keys are stored separately in `auth.json` and not in the config file. You can set up authentication via the TUI with the `/connect` command, or manually:

```json
{
  "providers": {
    "deepseek": {
      "api_key": "your-api-key-here"
    }
  }
}
```
