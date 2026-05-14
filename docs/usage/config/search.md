# Web Search Configuration

tidev's `websearch` tool supports multiple search providers. By default it uses
**Exa** (no API key required), but you can also use Brave Search, Google Custom
Search, or Tavily.

## Configuration

### `config.toml`

```toml
[websearch]
# Default provider used when the `provider` parameter is not specified.
# Supported: exa, brave, google, tavily
default_provider = "exa"

# Per-provider optional settings.
[websearch.providers.exa]
# Custom endpoint for a self-hosted Exa MCP server (default is the official
# public endpoint at https://mcp.exa.ai/mcp).
# endpoint = "https://mcp.exa.ai/mcp"

[websearch.providers.brave]
# Brave Search has no provider-specific config.
# API key goes in auth.json (see below).

[websearch.providers.google]
# Google Custom Search has no provider-specific config.
# API key and Search Engine ID go in auth.json.

[websearch.providers.tavily]
# Tavily has no provider-specific config.
# API key goes in auth.json.
```

### `auth.json` (API keys)

API keys are stored in `~/.local/share/tidev/auth.json`, separate from the main
config file. The structure looks like this:

```json
{
  "web": {
    "auth_token": null,
    "search_api_keys": {
      "brave": "BSA-your-brave-api-key",
      "google": "AIzaSy-your-google-api-key",
      "tavily": "tvly-your-tavily-api-key"
    },
    "google_cx": "your-google-search-engine-id"
  }
}
```

> **Note:** Exa does not require an API key for the public endpoint
> (`https://mcp.exa.ai/mcp`). The other providers require API keys to function.

## Provider reference

### Exa (`exa`)

| Item | Details |
|------|---------|
| API type | MCP / SSE |
| API key required | No (public endpoint) |
| Free tier | Rate-limited public endpoint |
| Endpoint | `https://mcp.exa.ai/mcp` (configurable) |
| Docs | https://docs.exa.ai/ |

`search_type` mapping:

| tidev value | Exa value |
|-------------|-----------|
| `"auto"` (default) | `"auto"` |
| `"fast"` | `"fast"` |
| `"deep"` | `"deep"` |

### Brave Search (`brave`)

| Item | Details |
|------|---------|
| API type | REST (JSON) |
| API key required | Yes (`X-Subscription-Token` header) |
| Free tier | 2,000 queries/month |
| Endpoint | `https://api.search.brave.com/res/v1/web/search` |
| Auth field | `web.search_api_keys.brave` |
| Sign up | https://api.search.brave.com/ |

`search_type` mapping:

| tidev value | Brave parameter |
|-------------|-----------------|
| `"auto"` (default) | No freshness filter |
| `"fast"` | `freshness=pw` (past week) |
| `"deep"` | No freshness filter |

### Google Custom Search (`google`)

| Item | Details |
|------|---------|
| API type | REST (JSON) |
| API key required | Yes (`key` query param) |
| CSE ID required | Yes (`cx` query param) |
| Free tier | 100 queries/day |
| Max results | 10 per request |
| Endpoint | `https://www.googleapis.com/customsearch/v1` |
| Auth fields | `web.search_api_keys.google`, `web.google_cx` |
| Sign up | https://developers.google.com/custom-search/v1/overview |

`search_type` mapping:

| tidev value | Google parameter |
|-------------|-----------------|
| `"auto"` (default) | Default sorting (by relevance) |
| `"fast"` | `sort=date` |
| `"deep"` | Default sorting |

> **Note:** Google Custom Search requires you to create a Programmable Search
> Engine and obtain its Search Engine ID (cx) from the control panel.

### Tavily (`tavily`)

| Item | Details |
|------|---------|
| API type | REST (JSON) |
| API key required | Yes (in POST body) |
| Free tier | 1,000 requests/month |
| Endpoint | `https://api.tavily.com/search` |
| Auth field | `web.search_api_keys.tavily` |
| Docs | https://docs.tavily.com/ |
| Sign up | https://tavily.com/ |

`search_type` mapping:

| tidev value | Tavily parameter |
|-------------|------------------|
| `"auto"` (default) | `search_depth=advanced` |
| `"fast"` | `search_depth=basic` |
| `"deep"` | `search_depth=advanced` |

Tavily also returns a human-readable `answer` field (an AI-generated summary
of the results), which is included at the top of the output.

## Usage from the LLM

When the LLM calls the `websearch` tool, it can specify the provider explicitly:

```json
{
  "query": "latest Rust news",
  "provider": "brave",
  "num_results": 5,
  "search_type": "fast"
}
```

If `provider` is omitted, the configured `default_provider` is used (defaults to
`"exa"`).

## Environment variables

- `WEBTOOLS_EXA_URL` — Override the Exa MCP endpoint URL (legacy, equivalent to
  setting `[websearch.providers.exa] endpoint` in config).
