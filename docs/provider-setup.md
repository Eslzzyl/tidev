# Provider Setup

TiDev can connect to providers directly from the TUI.

## Flow

1. Run `/connect`.
2. Type in the search box to filter providers by id or display name.
3. Select an existing provider and enter its API key.
4. Or select `Add new provider: ...` to launch the provider wizard.
5. Fill in the provider fields, enter the API key, then add one or more models.
6. After each model, TiDev asks whether you want to add another one.
7. TiDev saves the provider definition in `config.toml` and the key in `auth.json`.

## Wizard fields

- Provider id
- Provider display name
- Base URL
- API key
- Model id
- Model display name
- Context window
- Max output tokens
- Temperature

The wizard is intentionally OpenAI-compatible first. That keeps the happy path simple and lines up with the current HTTP client implementation.

## Notes

- New providers are created in the same TUI session, without editing TOML by hand.
- Bundled preset providers ship with TiDev, are merged at startup, and are labeled `preset` in the picker.
- The picker always keeps an `Add new provider` entry at the bottom so you can create a provider without leaving the panel.
- Existing providers can still be connected by selecting them in the filtered list.
- Session mode is controlled separately from provider setup.
