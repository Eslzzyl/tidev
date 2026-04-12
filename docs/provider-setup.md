# Provider Setup

TiDev can connect to providers directly from the TUI.

## Flow

1. Run `/connect`.
2. Choose an existing provider or select `Create new provider`.
3. Fill in the provider wizard.
4. Enter the API key for the new provider.
5. TiDev saves the provider definition in `config.toml` and the key in `auth.json`.

## Wizard fields

- Provider id
- Provider display name
- Base URL
- API key env var, if you want env-based fallback
- Model id
- Model display name
- Context window
- Max output tokens
- Temperature
- Prompt preset

The wizard is intentionally OpenAI-compatible first. That keeps the happy path simple and lines up with the current HTTP client implementation.

## Notes

- New providers are created in the same TUI session, without editing TOML by hand.
- Existing providers can still be connected by selecting them in the picker.
- The provider wizard uses the built-in provider setup prompt preset for consistent guidance.
