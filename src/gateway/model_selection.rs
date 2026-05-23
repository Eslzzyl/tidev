//! Shared model selection state machine for all gateway channels.
//!
//! This module provides a generic 4-step interactive model selection flow:
//!   1. Show current config → pick target (Chat / Agent / Memory)
//!   2. Pick provider
//!   3. Pick model
//!   4. If model supports thinking levels → pick thinking level
//!
//! Each channel implements [`ModelSelectionIO`] to bridge channel-specific IO.

use anyhow::Result;
use async_trait::async_trait;
use std::hash::Hash;

use crate::config::{ActiveModel, AppConfig, AuthStore, ConfigPaths};
use crate::storage::SessionStore;

use super::shared::{
    ModelSelectionTarget, all_target_entries, parse_target, target_display_name,
    thinking_options_for_model_id,
};

/// Interactive model selection state for a channel.
#[derive(Debug, Clone)]
pub enum ModelSelectionState {
    /// Legacy: waiting for user to select a provider.
    WaitingForProvider,
    /// Legacy: waiting for user to select a model for the given provider.
    WaitingForModel { provider_id: String },

    /// Step 1: waiting for user to select which target to configure.
    WaitingForTarget,
    /// Step 2: waiting for user to select a provider for the given target.
    WaitingForProviderWithTarget { target: ModelSelectionTarget },
    /// Step 3: waiting for user to select a model.
    WaitingForModelWithTarget { target: ModelSelectionTarget, provider_id: String },
    /// Step 4 (optional): waiting for user to select a thinking level.
    WaitingForThinkingLevel {
        target: ModelSelectionTarget,
        provider_id: String,
        model_id: String,
        model_label: String,
        thinking_options: Vec<String>,
    },
}

/// Abstract interface for channel-specific model selection IO.
///
/// Each gateway channel (QQ, Telegram) implements this trait to bridge
/// the shared state machine logic with its own message sending and state storage.
#[async_trait]
pub trait ModelSelectionIO: Send {
    /// The conversation identifier type used for state map lookups.
    /// QQ uses `String` (channel_id), Telegram uses `i64` (chat.id).
    type Id: Clone + Hash + Eq + Send + 'static;

    /// Send a text message to the conversation identified by `id`.
    async fn send_message(&mut self, id: &Self::Id, text: &str) -> Result<()>;

    /// Get the current model selection state for this conversation.
    fn get_state(&self, id: &Self::Id) -> Option<ModelSelectionState>;

    /// Set the model selection state for this conversation.
    fn set_state(&mut self, id: Self::Id, state: ModelSelectionState);

    /// Remove the model selection state for this conversation.
    fn remove_state(&mut self, id: &Self::Id);

    /// Build the persistence chat key (e.g. "qq:12345" or "telegram:67890").
    fn chat_key(&self, id: &Self::Id) -> String;

    /// Platform identifier ("qq" or "telegram").
    fn platform(&self) -> &'static str;

    // ── Resource accessors ──

    fn config(&self) -> &AppConfig;
    fn config_mut(&mut self) -> &mut AppConfig;
    fn config_paths(&self) -> &ConfigPaths;
    fn auth(&self) -> &AuthStore;
    fn store(&self) -> &SessionStore;

    /// Get available providers (with valid auth) as (provider_id, display_name).
    fn get_available_providers(&self) -> Vec<(String, String)>;

    /// Get models for a provider as (model_id, display_name).
    fn get_models_for_provider(&self, provider_id: &str) -> Vec<(String, String)>;

    /// Resolve the current chat model for this conversation.
    fn resolve_chat_model(&self, chat_key: &str) -> Result<ActiveModel>;
}

// ── Shared handler functions ──

/// Handle the /model command: show current config overview and start target selection.
pub async fn start_model_selection<IO: ModelSelectionIO>(
    io: &mut IO,
    id: &IO::Id,
) -> Result<()> {
    let overview = format_model_config_overview(io, id);
    let entries = all_target_entries();

    let mut text = format!("{}\n\nSelect what to configure (enter number):\n", overview);
    for (i, (_key, display)) in entries.iter().enumerate() {
        text.push_str(&format!("{}. {}\n", i + 1, display));
    }
    text.push_str("\n(Enter any other number to cancel)");

    io.send_message(id, &text).await?;
    io.set_state(id.clone(), ModelSelectionState::WaitingForTarget);
    Ok(())
}

/// Format a summary of the current model configuration for display.
fn format_model_config_overview<IO: ModelSelectionIO>(io: &IO, id: &IO::Id) -> String {
    let chat_key = io.chat_key(id);

    let chat_label = io
        .resolve_chat_model(&chat_key)
        .map(|m| m.label())
        .unwrap_or_else(|_| "unknown".to_string());

    let mut lines = vec!["📋 Current Model Configuration".to_string()];
    lines.push("────────────────────────────────".to_string());
    lines.push(format!("🟢 Chat:              {}", chat_label));

    for (entry_key, display_name) in all_target_entries() {
        if entry_key == "chat" || entry_key.starts_with("memory:") {
            continue;
        }
        if let Some(agent_type) = entry_key.strip_prefix("agent:") {
            let label = io
                .config()
                .agent_model_label(agent_type)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("<inherit> → {}", chat_label));
            lines.push(format!("🔹 {:<18} {}", display_name, label));
        }
    }

    let mem_label = io
        .config()
        .memory_model_label("consolidation")
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("<inherit> → {}", chat_label));
    lines.push("🔹 Memory:".to_string());
    lines.push(format!("   └ Consolidation:   {}", mem_label));

    lines.join("\n")
}

/// Handle one step of the model selection state machine.
pub async fn handle_step<IO: ModelSelectionIO>(
    io: &mut IO,
    id: &IO::Id,
    state: &ModelSelectionState,
    content: &str,
) -> Result<()> {
    // Any slash command cancels the selection
    if content.starts_with('/') {
        io.remove_state(id);
        io.send_message(id, "Selection cancelled. Send /model to try again.")
            .await?;
        return Ok(());
    }

    match state {
        ModelSelectionState::WaitingForTarget => {
            handle_target_selection(io, id, content).await
        }
        ModelSelectionState::WaitingForProviderWithTarget { target } => {
            handle_provider_selection(io, id, target, content).await
        }
        ModelSelectionState::WaitingForModelWithTarget { target, provider_id } => {
            handle_model_selection(io, id, target, provider_id, content).await
        }
        ModelSelectionState::WaitingForThinkingLevel {
            target,
            provider_id,
            model_id,
            thinking_options,
            ..
        } => {
            handle_thinking_level_selection(
                io, id, target, provider_id, model_id, thinking_options, content,
            )
            .await
        }

        // Legacy states (kept for backward compatibility)
        ModelSelectionState::WaitingForProvider => {
            handle_legacy_provider_selection(io, id, content).await
        }
        ModelSelectionState::WaitingForModel { provider_id } => {
            handle_legacy_model_selection(io, id, provider_id, content).await
        }
    }
}

// ── Step handlers ──

/// Step 1: pick target (Chat / Agent / Memory).
async fn handle_target_selection<IO: ModelSelectionIO>(
    io: &mut IO,
    id: &IO::Id,
    content: &str,
) -> Result<()> {
    let entries = all_target_entries();
    let selection: usize = match content.parse() {
        Ok(n) => n,
        Err(_) => {
            io.remove_state(id);
            io.send_message(id, "Invalid selection. Selection cancelled. Send /model to try again.")
                .await?;
            return Ok(());
        }
    };

    if selection < 1 || selection > entries.len() {
        io.remove_state(id);
        io.send_message(id, "Selection cancelled. Send /model to try again.")
            .await?;
        return Ok(());
    }

    let (entry_key, _display_name) = entries[selection - 1];
    let target = match parse_target(entry_key) {
        Some(t) => t,
        None => {
            io.remove_state(id);
            io.send_message(id, "Invalid target. Selection cancelled.")
                .await?;
            return Ok(());
        }
    };

    let providers = io.get_available_providers();
    if providers.is_empty() {
        io.remove_state(id);
        io.send_message(
            id,
            "No available providers found. Please check your configuration.",
        )
        .await?;
        return Ok(());
    }

    let mut text = String::from("Select a provider (enter number):\n\n");
    for (i, (_pid, display)) in providers.iter().enumerate() {
        text.push_str(&format!("{}. {}\n", i + 1, display));
    }
    text.push_str("\n(Enter any other number to cancel)");

    io.send_message(id, &text).await?;
    io.set_state(
        id.clone(),
        ModelSelectionState::WaitingForProviderWithTarget { target },
    );
    Ok(())
}

/// Step 2: pick a provider for the given target.
async fn handle_provider_selection<IO: ModelSelectionIO>(
    io: &mut IO,
    id: &IO::Id,
    target: &ModelSelectionTarget,
    content: &str,
) -> Result<()> {
    let providers = io.get_available_providers();
    let selection: usize = match content.parse() {
        Ok(n) => n,
        Err(_) => {
            io.remove_state(id);
            io.send_message(id, "Invalid selection. Selection cancelled. Send /model to try again.")
                .await?;
            return Ok(());
        }
    };

    if selection < 1 || selection > providers.len() {
        io.remove_state(id);
        io.send_message(id, "Selection cancelled. Send /model to try again.")
            .await?;
        return Ok(());
    }

    let (provider_id, _provider_name) = &providers[selection - 1];
    let models = io.get_models_for_provider(provider_id);
    if models.is_empty() {
        io.remove_state(id);
        io.send_message(id, "No models available for this provider. Selection cancelled.")
            .await?;
        return Ok(());
    }

    let mut text = format!("Select a model for {} (enter number):\n\n", provider_id);
    for (i, (_mid, display)) in models.iter().enumerate() {
        text.push_str(&format!("{}. {}\n", i + 1, display));
    }
    text.push_str("\n(Enter any other number to cancel)");

    io.send_message(id, &text).await?;
    io.set_state(
        id.clone(),
        ModelSelectionState::WaitingForModelWithTarget {
            target: target.clone(),
            provider_id: provider_id.clone(),
        },
    );
    Ok(())
}

/// Step 3: pick a model, then optionally ask for thinking level.
async fn handle_model_selection<IO: ModelSelectionIO>(
    io: &mut IO,
    id: &IO::Id,
    target: &ModelSelectionTarget,
    provider_id: &str,
    content: &str,
) -> Result<()> {
    let models = io.get_models_for_provider(provider_id);
    let selection: usize = match content.parse() {
        Ok(n) => n,
        Err(_) => {
            io.remove_state(id);
            io.send_message(id, "Invalid selection. Selection cancelled. Send /model to try again.")
                .await?;
            return Ok(());
        }
    };

    if selection < 1 || selection > models.len() {
        io.remove_state(id);
        io.send_message(id, "Invalid selection. Selection cancelled. Send /model to try again.")
            .await?;
        return Ok(());
    }

    let (model_id, _model_name) = &models[selection - 1];
    let model_label = format!("{}/{}", provider_id, model_id);

    let tl_options = thinking_options_for_model_id(model_id);
    if tl_options.is_empty() {
        // No thinking level support → save directly
        save_selection(io, id, target, provider_id, model_id, "").await?;
    } else {
        let mut text = format!(
            "Model {} supports thinking levels.\nSelect thinking level (enter number):\n\n",
            model_label,
        );
        for (i, opt) in tl_options.iter().enumerate() {
            // Strip prefix for cleaner display (e.g. "deepseek:High" → "High")
            let display = opt.split(':').next_back().unwrap_or(opt);
            text.push_str(&format!("{}. {}\n", i + 1, display));
        }
        text.push_str("\n(Enter any other number to skip / use auto-detect)");

        io.send_message(id, &text).await?;
        io.set_state(
            id.clone(),
            ModelSelectionState::WaitingForThinkingLevel {
                target: target.clone(),
                provider_id: provider_id.to_string(),
                model_id: model_id.to_string(),
                model_label,
                thinking_options: tl_options.iter().map(|s| s.to_string()).collect(),
            },
        );
    }
    Ok(())
}

/// Step 4: pick a thinking level and save.
async fn handle_thinking_level_selection<IO: ModelSelectionIO>(
    io: &mut IO,
    id: &IO::Id,
    target: &ModelSelectionTarget,
    provider_id: &str,
    model_id: &str,
    thinking_options: &[String],
    content: &str,
) -> Result<()> {
    let tl = match content.parse::<usize>() {
        Ok(n) if n >= 1 && n <= thinking_options.len() => thinking_options[n - 1].clone(),
        _ => String::new(), // skip → auto-detect
    };
    save_selection(io, id, target, provider_id, model_id, &tl).await
}

/// Persist the model (and optional thinking level) selection for the given target.
async fn save_selection<IO: ModelSelectionIO>(
    io: &mut IO,
    id: &IO::Id,
    target: &ModelSelectionTarget,
    provider_id: &str,
    model_id: &str,
    thinking_level: &str,
) -> Result<()> {
    match target {
        ModelSelectionTarget::Chat => {
            let chat_key = io.chat_key(id);
            io.store()
                .set_gateway_chat_model(io.platform(), &chat_key, provider_id, model_id)?;
            if !thinking_level.is_empty() {
                let _ = io
                    .store()
                    .save_model_thinking_level(provider_id, model_id, thinking_level);
            }
        }
        ModelSelectionTarget::Agent { agent_type } => {
            let model_str = format!("{}/{}", provider_id, model_id);
            let paths = io.config_paths().clone();
            io.config_mut().set_agent_model_and_thinking(
                &paths,
                agent_type,
                &model_str,
                thinking_level,
            )?;
        }
        ModelSelectionTarget::Memory { role } => {
            let model_str = format!("{}/{}", provider_id, model_id);
            let paths = io.config_paths().clone();
            io.config_mut().set_memory_model_and_thinking(
                &paths,
                role,
                &model_str,
                thinking_level,
            )?;
        }
    }

    io.remove_state(id);

    let target_name = target_display_name(target);
    let tl_suffix = if thinking_level.is_empty() {
        String::new()
    } else {
        format!(" ({})", thinking_level)
    };
    let model_label = format!("{}/{}", provider_id, model_id);
    let success_text = format!(
        "✅ Model switched to {}{} for {}\n\nSend /model to change again.",
        model_label, tl_suffix, target_name,
    );
    io.send_message(id, &success_text).await?;
    Ok(())
}

// ── Legacy handlers (backward-compatible entry via WaitingForProvider / WaitingForModel) ──

/// Legacy provider selection (no target step) — saves to chat model.
async fn handle_legacy_provider_selection<IO: ModelSelectionIO>(
    io: &mut IO,
    id: &IO::Id,
    content: &str,
) -> Result<()> {
    let providers = io.get_available_providers();
    let selection: usize = match content.parse() {
        Ok(n) => n,
        Err(_) => {
            io.remove_state(id);
            io.send_message(id, "Invalid selection. Selection cancelled. Send /model to try again.")
                .await?;
            return Ok(());
        }
    };

    if selection < 1 || selection > providers.len() {
        io.remove_state(id);
        io.send_message(id, "Selection cancelled. Send /model to try again.")
            .await?;
        return Ok(());
    }

    let (provider_id, _) = &providers[selection - 1];
    let models = io.get_models_for_provider(provider_id);
    if models.is_empty() {
        io.remove_state(id);
        io.send_message(id, "No models available for this provider. Selection cancelled.")
            .await?;
        return Ok(());
    }

    let mut text = format!("Select a model for {} (enter number):\n\n", provider_id);
    for (i, (_mid, display)) in models.iter().enumerate() {
        text.push_str(&format!("{}. {}\n", i + 1, display));
    }
    text.push_str("\n(Enter any other number to cancel)");

    io.send_message(id, &text).await?;
    io.set_state(
        id.clone(),
        ModelSelectionState::WaitingForModel {
            provider_id: provider_id.clone(),
        },
    );
    Ok(())
}

/// Legacy model selection — saves to chat model (with optional TL step).
async fn handle_legacy_model_selection<IO: ModelSelectionIO>(
    io: &mut IO,
    id: &IO::Id,
    provider_id: &str,
    content: &str,
) -> Result<()> {
    let models = io.get_models_for_provider(provider_id);
    let selection: usize = match content.parse() {
        Ok(n) => n,
        Err(_) => {
            io.remove_state(id);
            io.send_message(id, "Invalid selection. Selection cancelled. Send /model to try again.")
                .await?;
            return Ok(());
        }
    };

    if selection < 1 || selection > models.len() {
        io.remove_state(id);
        io.send_message(id, "Invalid selection. Selection cancelled. Send /model to try again.")
            .await?;
        return Ok(());
    }

    let (model_id, _) = &models[selection - 1];
    let model_label = format!("{}/{}", provider_id, model_id);

    let tl_options = thinking_options_for_model_id(model_id);
    if tl_options.is_empty() {
        // No TL support → save directly as chat model
        let chat_key = io.chat_key(id);
        io.store()
            .set_gateway_chat_model(io.platform(), &chat_key, provider_id, model_id)?;
        io.remove_state(id);
        let success_text = format!(
            "✅ Model switched to {}\n\nSend /model to change again.",
            model_label,
        );
        io.send_message(id, &success_text).await?;
    } else {
        let mut text = format!(
            "Model {} supports thinking levels.\nSelect thinking level (enter number):\n\n",
            model_label,
        );
        for (i, opt) in tl_options.iter().enumerate() {
            let display = opt.split(':').next_back().unwrap_or(opt);
            text.push_str(&format!("{}. {}\n", i + 1, display));
        }
        text.push_str("\n(Enter any other number to skip / use auto-detect)");

        io.send_message(id, &text).await?;
        io.set_state(
            id.clone(),
            ModelSelectionState::WaitingForThinkingLevel {
                target: ModelSelectionTarget::Chat,
                provider_id: provider_id.to_string(),
                model_id: model_id.to_string(),
                model_label,
                thinking_options: tl_options.iter().map(|s| s.to_string()).collect(),
            },
        );
    }
    Ok(())
}
