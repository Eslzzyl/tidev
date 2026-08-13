//! Settings panel with categorized navigation and immediate persistence.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::action::{Action, OverlayAction, OverlayKind, SettingKey, SettingValue, SettingsAction};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::theme::ThemePalette;
use crate::utils::centered_rect;

#[derive(Clone, Debug)]
pub(crate) enum SettingType {
    Toggle(bool),
    Number {
        value: f32,
        min: f32,
        max: f32,
    },
    Cycle {
        options: Vec<String>,
        selected: usize,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct SettingItem {
    pub name: String,
    pub description: String,
    pub setting_type: SettingType,
    pub key: SettingKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CategoryId {
    Interface,
    Notifications,
    Logging,
    Security,
    Agents,
}

struct SettingCategory {
    id: CategoryId,
    name: &'static str,
    items: Vec<SettingItem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusPane {
    Categories,
    Settings,
}

struct DropdownState {
    key: SettingKey,
    selected: usize,
    options: Vec<String>,
}

struct PanelLayout {
    overlay: Rect,
    inner: Rect,
    body: Rect,
    left: Rect,
    right: Rect,
    category_list: Rect,
    settings_list: Rect,
    detail: Rect,
}

pub(crate) struct SettingsPanel {
    category_index: usize,
    item_index: usize,
    focus: FocusPane,
    categories: Vec<SettingCategory>,
    dropdown: Option<DropdownState>,
}

impl SettingsPanel {
    pub(crate) fn new(config: &tidev_config::AppConfig) -> Self {
        let log_levels = vec![
            String::from("DEBUG"),
            String::from("INFO"),
            String::from("WARN"),
            String::from("ERROR"),
        ];
        let log_level_index = log_levels
            .iter()
            .position(|level| level == &config.logging.level.to_uppercase())
            .unwrap_or(1);

        let categories = vec![
            SettingCategory {
                id: CategoryId::Interface,
                name: "Interface",
                items: vec![
                    SettingItem {
                        name: String::from("Scroll speed"),
                        description: String::from("Scroll speed multiplier for chat navigation"),
                        setting_type: SettingType::Number {
                            value: config.ui.scroll_speed,
                            min: 1.0,
                            max: 10.0,
                        },
                        key: SettingKey::ScrollSpeed,
                    },
                    SettingItem {
                        name: String::from("Collapse thinking"),
                        description: String::from(
                            "Collapse thinking content by default for newly rendered messages",
                        ),
                        setting_type: SettingType::Toggle(config.ui.collapse_thinking),
                        key: SettingKey::CollapseThinking,
                    },
                    SettingItem {
                        name: String::from("Collapse diffs"),
                        description: String::from(
                            "Collapse edit, write, and patch diffs by default",
                        ),
                        setting_type: SettingType::Toggle(config.ui.collapse_diffs),
                        key: SettingKey::CollapseDiffs,
                    },
                    SettingItem {
                        name: String::from("Send while busy"),
                        description: String::from(
                            "Choose whether messages wait in a queue or steer the running turn",
                        ),
                        setting_type: SettingType::Cycle {
                            options: vec![String::from("queue"), String::from("steer")],
                            selected: match config.ui.send_while_busy {
                                tidev_config::SendWhileBusy::Queue => 0,
                                tidev_config::SendWhileBusy::Steer => 1,
                            },
                        },
                        key: SettingKey::SendWhileBusy,
                    },
                ],
            },
            SettingCategory {
                id: CategoryId::Notifications,
                name: "Notifications",
                items: vec![SettingItem {
                    name: String::from("Desktop notifications"),
                    description: String::from("Show terminal or desktop notifications"),
                    setting_type: SettingType::Toggle(config.notifications.enabled),
                    key: SettingKey::NotificationEnabled,
                }],
            },
            SettingCategory {
                id: CategoryId::Logging,
                name: "Logging",
                items: vec![
                    SettingItem {
                        name: String::from("Logging"),
                        description: String::from(
                            "Enable writing debug logs to the tidev data directory",
                        ),
                        setting_type: SettingType::Toggle(config.logging.enabled),
                        key: SettingKey::LoggingEnabled,
                    },
                    SettingItem {
                        name: String::from("Log level"),
                        description: String::from("Set the minimum level written by the logger"),
                        setting_type: SettingType::Cycle {
                            options: log_levels,
                            selected: log_level_index,
                        },
                        key: SettingKey::LogLevel,
                    },
                    SettingItem {
                        name: String::from("Save request body"),
                        description: String::from(
                            "Save serialized LLM request bodies for debugging",
                        ),
                        setting_type: SettingType::Toggle(config.logging.save_request_body),
                        key: SettingKey::SaveRequestBody,
                    },
                    SettingItem {
                        name: String::from("Save response body"),
                        description: String::from("Save raw LLM response payloads for debugging"),
                        setting_type: SettingType::Toggle(config.logging.save_response_body),
                        key: SettingKey::SaveResponseBody,
                    },
                ],
            },
            SettingCategory {
                id: CategoryId::Security,
                name: "Security",
                items: vec![
                    SettingItem {
                        name: String::from("Allow sensitive file access"),
                        description: String::from(
                            "Allow reading sensitive files without confirmation",
                        ),
                        setting_type: SettingType::Toggle(
                            config.access_control.allow_sensitive_file_access,
                        ),
                        key: SettingKey::AllowSensitiveFileAccess,
                    },
                    SettingItem {
                        name: String::from("Allow outside workspace access"),
                        description: String::from(
                            "Allow accessing files outside the workspace without confirmation",
                        ),
                        setting_type: SettingType::Toggle(
                            config.access_control.allow_outside_workspace_access,
                        ),
                        key: SettingKey::AllowOutsideWorkspaceAccess,
                    },
                ],
            },
            SettingCategory {
                id: CategoryId::Agents,
                name: "Agents",
                items: vec![SettingItem {
                    name: String::from("Subagent"),
                    description: String::from("Allow the task tool to spawn subagents"),
                    setting_type: SettingType::Toggle(config.subagent.enabled),
                    key: SettingKey::SubagentEnabled,
                }],
            },
        ];

        Self {
            category_index: 0,
            item_index: 0,
            focus: FocusPane::Categories,
            categories,
            dropdown: None,
        }
    }

    fn layout(area: Rect) -> PanelLayout {
        let overlay = centered_rect(area.width.min(100), area.height.saturating_sub(2), area);
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        let body = Rect::new(
            inner.x,
            inner.y + 2,
            inner.width,
            inner.height.saturating_sub(3),
        );
        let columns = Layout::horizontal([
            Constraint::Percentage(30),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(body);
        let left = columns[0];
        let right = columns[2];

        let category_list = Rect::new(
            left.x,
            left.y + 2,
            left.width,
            left.height.saturating_sub(2),
        );
        let settings_content = Rect::new(
            right.x,
            right.y + 2,
            right.width,
            right.height.saturating_sub(2),
        );
        let detail_height = settings_content.height.min(3);
        let settings_list = Rect::new(
            settings_content.x,
            settings_content.y,
            settings_content.width,
            settings_content.height.saturating_sub(detail_height),
        );
        let detail = Rect::new(
            settings_content.x,
            settings_content.y + settings_list.height,
            settings_content.width,
            detail_height,
        );

        PanelLayout {
            overlay,
            inner,
            body,
            left,
            right,
            category_list,
            settings_list,
            detail,
        }
    }

    fn current_category(&self) -> Option<&SettingCategory> {
        self.categories.get(self.category_index)
    }

    fn current_item(&self) -> Option<&SettingItem> {
        self.current_category()?.items.get(self.item_index)
    }

    fn move_category(&mut self, delta: isize) {
        if self.categories.is_empty() {
            return;
        }
        let len = self.categories.len() as isize;
        self.category_index = (self.category_index as isize + delta).rem_euclid(len) as usize;
        self.item_index = 0;
    }

    fn move_item(&mut self, delta: isize) {
        let Some(category) = self.current_category() else {
            return;
        };
        if category.items.is_empty() {
            return;
        }
        let len = category.items.len() as isize;
        self.item_index = (self.item_index as isize + delta).rem_euclid(len) as usize;
    }

    fn setting_change(&self, value: SettingValue) -> Option<Action> {
        let key = self.current_item()?.key;
        Some(Action::Settings(SettingsAction::Change { key, value }))
    }

    fn toggle_current(&self) -> Option<Action> {
        let SettingType::Toggle(value) = &self.current_item()?.setting_type else {
            return None;
        };
        self.setting_change(SettingValue::Bool(!*value))
    }

    fn adjust_current(&self, delta: isize) -> Option<Action> {
        let item = self.current_item()?;
        let value = match &item.setting_type {
            SettingType::Number { value, min, max } => {
                SettingValue::Number((*value + delta as f32).clamp(*min, *max))
            }
            SettingType::Cycle { options, selected } if !options.is_empty() => {
                let len = options.len() as isize;
                let index = (*selected as isize + delta).rem_euclid(len) as usize;
                SettingValue::Choice(options[index].clone())
            }
            _ => return None,
        };
        self.setting_change(value)
    }

    fn open_dropdown(&mut self) {
        let Some(item) = self.current_item() else {
            return;
        };
        let SettingType::Cycle { options, selected } = &item.setting_type else {
            return;
        };
        self.dropdown = Some(DropdownState {
            key: item.key,
            selected: *selected,
            options: options.clone(),
        });
    }

    fn dropdown_action(&mut self) -> Option<Action> {
        let dropdown = self.dropdown.take()?;
        let value = dropdown.options.get(dropdown.selected)?.clone();
        Some(Action::Settings(SettingsAction::Change {
            key: dropdown.key,
            value: SettingValue::Choice(value),
        }))
    }

    fn update_item(&mut self, key: SettingKey, value: &SettingValue) {
        for category in &mut self.categories {
            for item in &mut category.items {
                if item.key != key {
                    continue;
                }
                match (&mut item.setting_type, value) {
                    (SettingType::Toggle(current), SettingValue::Bool(value)) => *current = *value,
                    (SettingType::Number { value: current, .. }, SettingValue::Number(value)) => {
                        *current = *value
                    }
                    (SettingType::Cycle { options, selected }, SettingValue::Choice(value)) => {
                        if let Some(index) = options.iter().position(|option| option == value) {
                            *selected = index;
                        }
                    }
                    _ => {}
                }
            }
        }
        self.dropdown = None;
    }

    fn visible_start(&self, height: usize) -> usize {
        let count = self.current_category().map(|c| c.items.len()).unwrap_or(0);
        let height = height.max(1);
        self.item_index
            .saturating_sub(height.saturating_sub(1))
            .min(count.saturating_sub(height))
    }

    fn dropdown_rect(&self, layout: &PanelLayout) -> Option<Rect> {
        let dropdown = self.dropdown.as_ref()?;
        let width = dropdown
            .options
            .iter()
            .map(|option| option.width())
            .max()
            .unwrap_or(4)
            .saturating_add(6) as u16;
        let width = width.min(layout.right.width.max(1));
        let start = self.visible_start(layout.settings_list.height as usize);
        let row_y = layout
            .settings_list
            .y
            .saturating_add(self.item_index.saturating_sub(start) as u16);
        let height = (dropdown.options.len() as u16).saturating_add(2);
        let y = if row_y.saturating_add(height) <= layout.right.bottom() {
            row_y
        } else {
            row_y.saturating_sub(height.saturating_sub(1))
        };
        let x = layout.right.right().saturating_sub(width);
        Some(Rect::new(x, y, width, height.min(layout.overlay.height)))
    }
}

impl Component for SettingsPanel {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        if let Some(dropdown) = &mut self.dropdown {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if !dropdown.options.is_empty() {
                        dropdown.selected = (dropdown.selected + dropdown.options.len() - 1)
                            % dropdown.options.len();
                    }
                    None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if !dropdown.options.is_empty() {
                        dropdown.selected = (dropdown.selected + 1) % dropdown.options.len();
                    }
                    None
                }
                KeyCode::Enter | KeyCode::Char(' ') => self.dropdown_action(),
                KeyCode::Esc | KeyCode::Left => {
                    self.dropdown = None;
                    None
                }
                _ => None,
            }
        } else {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.focus == FocusPane::Categories {
                        self.move_category(-1);
                    } else {
                        self.move_item(-1);
                    }
                    None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.focus == FocusPane::Categories {
                        self.move_category(1);
                    } else {
                        self.move_item(1);
                    }
                    None
                }
                KeyCode::Left => {
                    if self.focus == FocusPane::Settings {
                        if let Some(action) = self.adjust_current(-1) {
                            Some(action)
                        } else {
                            self.focus = FocusPane::Categories;
                            None
                        }
                    } else {
                        None
                    }
                }
                KeyCode::Right => {
                    if self.focus == FocusPane::Categories {
                        self.focus = FocusPane::Settings;
                        None
                    } else {
                        self.adjust_current(1)
                    }
                }
                KeyCode::Tab => {
                    self.focus = match self.focus {
                        FocusPane::Categories => FocusPane::Settings,
                        FocusPane::Settings => FocusPane::Categories,
                    };
                    None
                }
                KeyCode::Enter => {
                    if self.focus == FocusPane::Categories {
                        self.focus = FocusPane::Settings;
                        None
                    } else {
                        match self.current_item().map(|item| &item.setting_type) {
                            Some(SettingType::Cycle { .. }) => {
                                self.open_dropdown();
                                None
                            }
                            Some(SettingType::Toggle(_)) => self.toggle_current(),
                            _ => None,
                        }
                    }
                }
                KeyCode::Char(' ') if self.focus == FocusPane::Settings => self.toggle_current(),
                KeyCode::Esc | KeyCode::Char('q') => Some(Action::Overlay(OverlayAction::Close(
                    OverlayKind::SettingsPanel,
                ))),
                _ => None,
            }
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Option<Action> {
        let position = Position::new(mouse.column, mouse.row);
        let layout = Self::layout(area);
        if !layout.overlay.contains(position) {
            return None;
        }

        if let Some(dropdown_rect) = self.dropdown_rect(&layout) {
            if dropdown_rect.contains(position) {
                if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                    let row = position.y.saturating_sub(dropdown_rect.y + 1) as usize;
                    if let Some(dropdown) = &mut self.dropdown
                        && row < dropdown.options.len()
                    {
                        dropdown.selected = row;
                        return self.dropdown_action();
                    }
                }
                return Some(Action::Consumed);
            }
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.dropdown = None;
                return Some(Action::Consumed);
            }
        }

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if layout.category_list.contains(position) {
                    self.move_category(-1);
                } else if layout.settings_list.contains(position) {
                    self.move_item(-1);
                }
                Some(Action::Consumed)
            }
            MouseEventKind::ScrollDown => {
                if layout.category_list.contains(position) {
                    self.move_category(1);
                } else if layout.settings_list.contains(position) {
                    self.move_item(1);
                }
                Some(Action::Consumed)
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if layout.category_list.contains(position) {
                    let index = position.y.saturating_sub(layout.category_list.y) as usize;
                    if index < self.categories.len() {
                        self.category_index = index;
                        self.item_index = 0;
                        self.focus = FocusPane::Categories;
                    }
                } else if layout.settings_list.contains(position) {
                    let start = self.visible_start(layout.settings_list.height as usize);
                    let index = start + position.y.saturating_sub(layout.settings_list.y) as usize;
                    if self
                        .current_category()
                        .is_some_and(|category| index < category.items.len())
                    {
                        self.item_index = index;
                        self.focus = FocusPane::Settings;
                        match self.current_item().map(|item| &item.setting_type) {
                            Some(SettingType::Toggle(_)) => return self.toggle_current(),
                            Some(SettingType::Cycle { .. }) => self.open_dropdown(),
                            _ => {}
                        }
                    }
                }
                Some(Action::Noop)
            }
            _ => Some(Action::Noop),
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        if let Action::Settings(SettingsAction::Change { key, value }) = action {
            self.update_item(*key, value);
        }
        vec![]
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let layout = Self::layout(rect);

        frame.render_widget(Clear, layout.overlay);
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.panel_alt)),
            layout.overlay,
        );

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                " Settings ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(layout.inner.x, layout.inner.y, layout.inner.width, 1),
        );

        let separator: Vec<Line> = (0..layout.body.height)
            .map(|_| Line::from(Span::styled("│", Style::default().fg(palette.border))))
            .collect();
        frame.render_widget(
            Paragraph::new(separator),
            Rect::new(layout.left.right(), layout.body.y, 1, layout.body.height),
        );

        for (area, title) in [
            (layout.left, "Categories"),
            (
                layout.right,
                self.current_category()
                    .map(|c| c.name)
                    .unwrap_or("Settings"),
            ),
        ] {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!("  {title}"),
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                )))
                .style(Style::default().bg(palette.panel_alt)),
                Rect::new(area.x, area.y, area.width, 1),
            );
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "─".repeat(area.width as usize),
                    Style::default().fg(palette.border),
                )))
                .style(Style::default().bg(palette.panel_alt)),
                Rect::new(area.x, area.y + 1, area.width, 1),
            );
        }

        let category_items: Vec<ListItem> = self
            .categories
            .iter()
            .map(|category| {
                let selected =
                    category.id == self.current_category().map(|c| c.id).unwrap_or(category.id);
                let style = if selected && self.focus == FocusPane::Categories {
                    Style::default()
                        .bg(palette.selection_bg)
                        .fg(palette.selection_fg)
                        .add_modifier(Modifier::BOLD)
                } else if selected {
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette.text)
                };
                ListItem::new(Line::from(Span::styled(
                    format!("  {}  {}", if selected { "›" } else { " " }, category.name),
                    style,
                )))
            })
            .collect();
        let mut category_state = ListState::default();
        category_state.select(Some(self.category_index));
        frame.render_stateful_widget(
            List::new(category_items)
                .style(Style::default().bg(palette.panel_alt))
                .highlight_style(
                    Style::default()
                        .bg(palette.selection_bg)
                        .fg(palette.selection_fg),
                ),
            layout.category_list,
            &mut category_state,
        );

        let start = self.visible_start(layout.settings_list.height as usize);
        let selected_key = self.current_item().map(|current| current.key);
        let focus = self.focus;
        let visible_lines: Vec<Line> = self
            .current_category()
            .map(|category| {
                category
                    .items
                    .iter()
                    .skip(start)
                    .take(layout.settings_list.height as usize)
                    .map(|item| {
                        format_setting_line(
                            item,
                            layout.settings_list.width as usize,
                            palette,
                            Some(item.key) == selected_key && focus == FocusPane::Settings,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        frame.render_widget(
            Paragraph::new(visible_lines).style(Style::default().bg(palette.panel_alt)),
            layout.settings_list,
        );

        if let Some(item) = self.current_item() {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(
                        format!("  {}", item.name),
                        Style::default()
                            .fg(palette.muted)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(Span::styled(
                        format!("  {}", item.description),
                        Style::default().fg(palette.muted),
                    )),
                ])
                .wrap(Wrap { trim: true })
                .style(Style::default().bg(palette.panel_alt)),
                layout.detail,
            );
        }

        let footer = "↑/↓ navigate  ←/→ change  Tab switch pane  Enter edit  Esc close";
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("  {footer}"),
                Style::default().fg(palette.muted),
            )))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(
                layout.inner.x,
                layout.inner.bottom().saturating_sub(1),
                layout.inner.width,
                1,
            ),
        );

        if let Some(dropdown_rect) = self.dropdown_rect(&layout) {
            frame.render_widget(Clear, dropdown_rect);
            let options = self
                .dropdown
                .as_ref()
                .map(|dropdown| {
                    dropdown
                        .options
                        .iter()
                        .map(|option| ListItem::new(option.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mut state = ListState::default();
            state.select(self.dropdown.as_ref().map(|dropdown| dropdown.selected));
            frame.render_stateful_widget(
                List::new(options)
                    .block(Block::bordered().style(Style::default().bg(palette.panel_alt)))
                    .highlight_style(
                        Style::default()
                            .bg(palette.selection_bg)
                            .fg(palette.selection_fg)
                            .add_modifier(Modifier::BOLD),
                    ),
                dropdown_rect,
                &mut state,
            );
        }
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
}

fn format_setting_line(
    item: &SettingItem,
    width: usize,
    palette: ThemePalette,
    selected: bool,
) -> Line<'static> {
    let base_style = if selected {
        Style::default()
            .bg(palette.selection_bg)
            .fg(palette.selection_fg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(palette.panel_alt).fg(palette.text)
    };
    let name = format!("  {}", item.name);
    let (value, value_style) = match &item.setting_type {
        // Keep both states five columns wide so adjacent rows stay aligned.
        SettingType::Toggle(true) => (
            String::from("● ON "),
            if selected {
                base_style
            } else {
                base_style.fg(palette.accent).add_modifier(Modifier::BOLD)
            },
        ),
        SettingType::Toggle(false) => (
            String::from("○ OFF"),
            if selected {
                base_style
            } else {
                base_style.fg(palette.muted)
            },
        ),
        SettingType::Number { value, .. } => (
            format!("{value:.1}×"),
            if selected {
                base_style
            } else {
                base_style.fg(palette.accent).add_modifier(Modifier::BOLD)
            },
        ),
        SettingType::Cycle {
            options,
            selected: selected_index,
        } => (
            format!(
                "▾ {}",
                options
                    .get(*selected_index)
                    .map(String::as_str)
                    .unwrap_or("?")
            ),
            if selected {
                base_style
            } else {
                base_style.fg(palette.accent).add_modifier(Modifier::BOLD)
            },
        ),
    };
    let gap = width
        .saturating_sub(name.width())
        .saturating_sub(value.width())
        .max(1);
    Line::from(vec![
        Span::styled(name, base_style),
        Span::styled(" ".repeat(gap), base_style),
        Span::styled(value, value_style),
    ])
}
