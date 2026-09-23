//! Startup dialog that guides users through connecting a provider.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use crate::action::{Action, OverlayAction, OverlayKind};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::i18n::TextKey;
use crate::utils::centered_rect;

pub(crate) fn has_connected_provider(
    config: &tidev_config::AppConfig,
    auth: &tidev_config::auth::AuthStore,
) -> bool {
    config
        .provider_ids()
        .iter()
        .any(|provider_id| auth.api_key(provider_id).is_some())
}

pub(crate) struct StartupProviderDialog;

impl StartupProviderDialog {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Component for StartupProviderDialog {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Esc => Some(Action::Overlay(OverlayAction::Close(
                OverlayKind::StartupProviderDialog,
            ))),
            _ => None,
        }
    }

    fn update(&mut self, _action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        Vec::new()
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let overlay = centered_rect(rect.width.min(76), rect.height.min(11), rect);
        frame.render_widget(Clear, overlay);
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.panel_alt)),
            overlay,
        );

        let inner = overlay.inner(Margin {
            horizontal: 2,
            vertical: 1,
        });
        let sections = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                ctx.ui_text.text(TextKey::StartupProviderTitle),
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

        frame.render_widget(
            Paragraph::new(ctx.ui_text.text(TextKey::StartupProviderMessage))
                .alignment(ratatui::layout::Alignment::Center)
                .wrap(Wrap { trim: true })
                .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
            sections[1],
        );

        frame.render_widget(
            Paragraph::new(ctx.ui_text.text(TextKey::StartupProviderFooter))
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[2],
        );
    }

    fn is_overlay(&self) -> bool {
        true
    }

    fn z_order(&self) -> u8 {
        10
    }

    fn blocks_input(&self) -> bool {
        true
    }

    fn captures_all_input(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn startup_detection_uses_keys_for_configured_providers() {
        let config = tidev_config::AppConfig::default();
        let provider_id = config
            .provider_ids()
            .into_iter()
            .next()
            .expect("bundled provider catalog contains providers");
        let mut auth = tidev_config::auth::AuthStore::default();

        assert!(!has_connected_provider(&config, &auth));

        auth.set_api_key(provider_id, "test-key");
        assert!(has_connected_provider(&config, &auth));
    }

    #[test]
    fn enter_dismisses_the_startup_dialog() {
        let mut dialog = StartupProviderDialog::new();

        let action = dialog.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            action,
            Some(Action::Overlay(OverlayAction::Close(
                OverlayKind::StartupProviderDialog
            )))
        ));
        assert!(dialog.blocks_input());
        assert!(dialog.captures_all_input());
    }
}
