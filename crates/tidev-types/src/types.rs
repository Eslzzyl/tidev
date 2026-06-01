//! Shared type definitions used across multiple tidev components.
//!
//! This module will become the foundation for the `tidev-types` crate
//! when the workspace is split.

use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;
// ── Permission types (originally split across config + tooling) ─────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermission {
    Read,
    Search,
    Write,
    Edit,
    Execute,
    Session,
}

impl ToolPermission {
    pub fn is_allowed_in(
        self,
        mode: crate::prompts::SessionMode,
        permission_config: &PermissionConfig,
    ) -> bool {
        permission_config.is_allowed(mode, self)
    }

    pub fn needs_confirmation(self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PermissionSettings {
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub search: bool,
    #[serde(default)]
    pub write: bool,
    #[serde(default)]
    pub edit: bool,
    #[serde(default)]
    pub execute: bool,
    #[serde(default)]
    pub session: bool,
}

impl PermissionSettings {
    pub fn is_allowed(&self, permission: ToolPermission) -> bool {
        match permission {
            ToolPermission::Read => self.read,
            ToolPermission::Search => self.search,
            ToolPermission::Write => self.write,
            ToolPermission::Edit => self.edit,
            ToolPermission::Execute => self.execute,
            ToolPermission::Session => self.session,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermissionConfig {
    #[serde(default)]
    pub plan: PermissionSettings,
    #[serde(default)]
    pub build: PermissionSettings,
}

impl PermissionConfig {
    pub fn is_allowed(
        &self,
        mode: crate::prompts::SessionMode,
        permission: ToolPermission,
    ) -> bool {
        match mode {
            crate::prompts::SessionMode::Plan => self.plan.is_allowed(permission),
            crate::prompts::SessionMode::Build => self.build.is_allowed(permission),
        }
    }
}

impl Default for PermissionConfig {
    fn default() -> Self {
        Self {
            plan: PermissionSettings {
                read: true,
                search: true,
                write: false,
                edit: false,
                execute: true,
                session: true,
            },
            build: PermissionSettings {
                read: true,
                search: true,
                write: true,
                edit: true,
                execute: true,
                session: true,
            },
        }
    }
}

// ── TodoItem (moved from tooling to break tooling↔storage cycle) ─────

/// A task/todo item within a session.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TodoItem {
    pub content: String,
    pub status: String,
}

// ── Goal (session-level persistent goal for /goal command) ───────────

/// The status of a session goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    /// Goal is active — continuation prompt is injected each turn.
    Active,
    /// Goal is paused — preserved but not injected.
    Paused,
    /// Goal has been marked complete by the model or user.
    Complete,
}

impl GoalStatus {
    /// Return the lowercase string representation used in the database.
    pub fn as_str(&self) -> &'static str {
        match self {
            GoalStatus::Active => "active",
            GoalStatus::Paused => "paused",
            GoalStatus::Complete => "complete",
        }
    }
}

impl FromStr for GoalStatus {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(GoalStatus::Active),
            "paused" => Ok(GoalStatus::Paused),
            "complete" => Ok(GoalStatus::Complete),
            _ => Err("unknown goal status"),
        }
    }
}

/// A persistent goal for a session, stored in `session_goals` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub session_id: Uuid,
    pub objective: String,
    pub status: GoalStatus,
    pub tokens_used: i64,
    pub time_used_seconds: i64,
    /// RFC3339 timestamp of creation.
    pub created_at: String,
    /// RFC3339 timestamp of last update.
    pub updated_at: String,
}
