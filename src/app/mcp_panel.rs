use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::mcp::McpServerSummary;

use super::App;

#[derive(Clone, Debug)]
pub struct McpPanelState {
    pub selected_index: usize,
}

impl McpPanelState {
    pub fn new() -> Self {
        Self { selected_index: 0 }
    }

    pub fn move_selection(&mut self, items: &[McpPanelItem], delta: isize) {
        if items.is_empty() {
            self.selected_index = 0;
            return;
        }

        let len = items.len() as isize;
        let current = self.selected_index.min(items.len().saturating_sub(1)) as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        self.selected_index = next;
    }

    pub fn selected_item<'a>(&self, items: &'a [McpPanelItem]) -> Option<&'a McpPanelItem> {
        items.get(self.selected_index)
    }
}

impl Default for McpPanelState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct McpPanelItem {
    pub summary: McpServerSummary,
}

impl McpPanelItem {
    pub fn is_selectable(&self) -> bool {
        true
    }
}

impl App {
    pub(crate) fn open_mcp_panel(&mut self, initial_query: String) {
        self.command_palette.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.composer.clear();
        self.composer
            .set_placeholder("Search MCP servers by name or transport");
        self.composer.set_text(initial_query);

        let mut panel = McpPanelState::new();
        let items = self.mcp_panel_items();
        panel.selected_index = first_selectable_index(&items).unwrap_or(0);
        self.mcp_panel = Some(panel);
    }

    pub(crate) fn close_mcp_panel(&mut self) {
        if self.mcp_panel.take().is_some() {
            self.composer.clear();
            self.composer
                .set_placeholder("Ask TiDev about your code, task, or question...");
        }
    }

    pub(crate) fn reset_mcp_panel_selection(&mut self) {
        let items = self.mcp_panel_items();
        if let Some(panel) = &mut self.mcp_panel {
            panel.selected_index = first_selectable_index(&items).unwrap_or(0);
        }
    }

    pub(crate) fn handle_mcp_panel_key(
        &mut self,
        key: KeyEvent,
        runtime: &tokio::runtime::Runtime,
    ) -> Result<()> {
        let Some(panel) = self.mcp_panel.clone() else {
            return Ok(());
        };

        let items = self.mcp_panel_items();

        match key.code {
            KeyCode::Up => {
                let mut next_panel = panel;
                next_panel.move_selection(&items, -1);
                self.mcp_panel = Some(next_panel);
            }
            KeyCode::Down => {
                let mut next_panel = panel;
                next_panel.move_selection(&items, 1);
                self.mcp_panel = Some(next_panel);
            }
            KeyCode::Enter => {
                if let Some(selected) = panel.selected_item(&items) {
                    let name = selected.summary.name.clone();
                    let result = match selected.summary.status {
                        crate::mcp::McpConnectionStatus::Connected
                        | crate::mcp::McpConnectionStatus::Connecting => {
                            runtime.block_on(self.tools.disconnect_mcp_server(&name))
                        }
                        _ => runtime.block_on(self.tools.toggle_mcp_server(&name)),
                    };

                    match result {
                        Ok(()) => {
                            self.last_notice = Some(format!("Updated MCP server '{name}'"));
                        }
                        Err(error) => {
                            self.last_notice = Some(error.to_string());
                        }
                    }
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                if let Some(selected) = panel.selected_item(&items) {
                    let name = selected.summary.name.clone();
                    if let Err(error) = runtime.block_on(self.tools.refresh_mcp_server(&name)) {
                        self.last_notice = Some(error.to_string());
                    } else {
                        self.last_notice = Some(format!("Refreshed MCP server '{name}'"));
                    }
                }
            }
            KeyCode::Esc => {
                self.close_mcp_panel();
            }
            KeyCode::Tab => {}
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let mut next_panel = panel;
                next_panel.move_selection(&items, -1);
                self.mcp_panel = Some(next_panel);
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let mut next_panel = panel;
                next_panel.move_selection(&items, 1);
                self.mcp_panel = Some(next_panel);
            }
            _ => {
                let previous_query = self.composer.text().to_string();
                let _ = self.composer.handle_key_with_history(key, false);
                if self.composer.text() != previous_query {
                    self.reset_mcp_panel_selection();
                }
            }
        }

        Ok(())
    }

    pub(crate) fn mcp_panel_items(&self) -> Vec<McpPanelItem> {
        let query = self.composer.text().trim().to_ascii_lowercase();
        self.tools
            .mcp_summaries()
            .into_iter()
            .filter(|summary| mcp_panel_matches_query(&query, summary))
            .map(|summary| McpPanelItem { summary })
            .collect()
    }
}

fn first_selectable_index(items: &[McpPanelItem]) -> Option<usize> {
    items.iter().position(McpPanelItem::is_selectable)
}

fn mcp_panel_matches_query(query: &str, summary: &McpServerSummary) -> bool {
    if query.is_empty() {
        return true;
    }

    let name = summary.name.to_ascii_lowercase();
    let kind = summary.kind.to_ascii_lowercase();
    let status = summary.status_text().to_ascii_lowercase();
    let tool_count = summary.tool_count.to_string();

    name.contains(query)
        || kind.contains(query)
        || status.contains(query)
        || tool_count.contains(query)
}
