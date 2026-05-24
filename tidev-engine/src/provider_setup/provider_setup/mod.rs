mod edit_provider;
mod new_provider;

use anyhow::{Context, Result, bail};

pub use edit_provider::{EditModelStep, EditProviderDraft, EditProviderStep};
pub use new_provider::{NewModelDraft, NewProviderDraft, NewProviderStep};

#[derive(Clone, Debug)]
pub enum ConnectDialog {
    ProviderPicker {
        selected: usize,
    },
    ApiKey {
        provider_id: String,
    },
    NewProvider {
        step: NewProviderStep,
        draft: NewProviderDraft,
    },
    EditProvider {
        provider_id: String,
        step: EditProviderStep,
        model_step: Option<EditModelStep>,
        draft: EditProviderDraft,
    },
}

impl ConnectDialog {
    pub fn provider_picker() -> Self {
        Self::ProviderPicker { selected: 0 }
    }

    pub fn api_key(provider_id: impl Into<String>) -> Self {
        Self::ApiKey {
            provider_id: provider_id.into(),
        }
    }

    pub fn new_provider() -> Self {
        Self::NewProvider {
            step: NewProviderStep::ProviderId,
            draft: NewProviderDraft::default(),
        }
    }
}

fn non_empty<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    if value.is_empty() {
        bail!("{label} cannot be empty");
    }

    Ok(value)
}

fn normalize_identifier(value: &str, label: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase().replace([' ', '.'], "-");

    if normalized.is_empty() {
        bail!("{label} cannot be empty");
    }

    if normalized
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_'))
    {
        Ok(normalized)
    } else {
        bail!("{label} may only contain lowercase letters, numbers, '-' or '_'");
    }
}

fn normalize_base_url(value: &str) -> Result<String> {
    let value = value.trim().trim_end_matches('/');

    if value.is_empty() {
        bail!("base URL cannot be empty");
    }

    if !(value.starts_with("http://") || value.starts_with("https://")) {
        bail!("base URL must start with http:// or https://");
    }

    Ok(value.to_string())
}

fn parse_usize(value: &str, label: &str) -> Result<usize> {
    value
        .parse::<usize>()
        .with_context(|| format!("{label} must be a positive integer"))
}

fn parse_temperature(value: &str) -> Result<f32> {
    let temperature = value
        .parse::<f32>()
        .with_context(|| "temperature must be a number")?;

    if !(0.0..=2.0).contains(&temperature) {
        bail!("temperature should usually be between 0.0 and 2.0");
    }

    Ok(temperature)
}
