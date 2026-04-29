use std::collections::HashMap;
use uuid::Uuid;

use crate::agent::AgentType;

/// Maximum delegation depth to prevent infinite recursion.
const DEFAULT_MAX_DEPTH: usize = 3;

/// Maximum pending tasks per parent session.
const DEFAULT_MAX_SESSIONS_PER_AGENT: usize = 5;

/// Tracks pending and in-flight sub-agent delegations.
///
/// Used to enforce delegation depth limits and prevent recursive delegation cycles.
#[derive(Clone, Debug)]
pub struct DelegateManager {
    /// Maximum delegation chain depth.
    max_depth: usize,
    /// Maximum pending tasks per parent session.
    max_sessions_per_agent: usize,
    /// In-flight sub-agent tasks: child_session_id -> PendingTaskInfo.
    active_tasks: HashMap<Uuid, PendingTaskInfo>,
    /// Depth cache: parent_session_id -> current delegation depth.
    depth_cache: HashMap<Uuid, usize>,
}

/// Information about a pending sub-agent task.
#[derive(Clone, Debug)]
pub struct PendingTaskInfo {
    pub child_session_id: Uuid,
    pub parent_session_id: Uuid,
    pub agent_type: AgentType,
    pub description: String,
    pub prompt: String,
}

impl Default for DelegateManager {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_DEPTH,
            max_sessions_per_agent: DEFAULT_MAX_SESSIONS_PER_AGENT,
            active_tasks: HashMap::new(),
            depth_cache: HashMap::new(),
        }
    }
}

impl DelegateManager {
    pub fn new(max_depth: usize, max_sessions_per_agent: usize) -> Self {
        Self {
            max_depth,
            max_sessions_per_agent,
            active_tasks: HashMap::new(),
            depth_cache: HashMap::new(),
        }
    }

    /// Check whether a delegation would exceed the depth limit.
    ///
    /// Returns `Ok(depth)` where `depth` is the current depth of the parent session
    /// (0-indexed, so depth 0 means the parent is the root orchestrator).
    pub fn check_depth(&mut self, parent_session_id: Uuid) -> anyhow::Result<usize> {
        let current_depth = self.depth_cache.get(&parent_session_id).copied().unwrap_or(0);
        if current_depth >= self.max_depth {
            anyhow::bail!(
                "Max delegation depth ({}) reached for session {}",
                self.max_depth,
                parent_session_id
            );
        }
        Ok(current_depth)
    }

    /// Track a new sub-agent task that is about to be started.
    ///
    /// Returns the depth of the child session.
    pub fn track_task(&mut self, info: PendingTaskInfo) -> anyhow::Result<usize> {
        // Check pending task limit
        let pending_count = self
            .active_tasks
            .values()
            .filter(|t| t.parent_session_id == info.parent_session_id)
            .count();
        if pending_count >= self.max_sessions_per_agent {
            anyhow::bail!(
                "Max pending tasks ({}) reached for session {}",
                self.max_sessions_per_agent,
                info.parent_session_id
            );
        }

        let parent_depth = self.depth_cache.get(&info.parent_session_id).copied().unwrap_or(0);
        let child_depth = parent_depth + 1;

        if child_depth > self.max_depth {
            anyhow::bail!(
                "Max delegation depth ({}) reached for session {}",
                self.max_depth,
                info.parent_session_id
            );
        }

        self.active_tasks.insert(info.child_session_id, info);
        Ok(child_depth)
    }

    /// Record the depth of a session (called when a session is created).
    pub fn record_depth(&mut self, session_id: Uuid, depth: usize) {
        self.depth_cache.insert(session_id, depth);
    }

    /// Mark a sub-agent task as completed and remove it from tracking.
    pub fn complete_task(&mut self, child_session_id: Uuid) -> Option<PendingTaskInfo> {
        self.active_tasks.remove(&child_session_id)
    }

    /// Get the depth of a session.
    pub fn get_depth(&self, session_id: Uuid) -> usize {
        self.depth_cache.get(&session_id).copied().unwrap_or(0)
    }

    /// Get the count of active tasks for a parent session.
    pub fn active_task_count(&self, parent_session_id: Uuid) -> usize {
        self.active_tasks
            .values()
            .filter(|t| t.parent_session_id == parent_session_id)
            .count()
    }

    /// Get all active tasks.
    pub fn all_active_tasks(&self) -> &HashMap<Uuid, PendingTaskInfo> {
        &self.active_tasks
    }

    /// Clean up stale entries (e.g., completed/cancelled sessions).
    pub fn cleanup(&mut self, active_session_ids: &[Uuid]) {
        self.active_tasks.retain(|id, _| active_session_ids.contains(id));
        self.depth_cache.retain(|id, _| active_session_ids.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_depth_tracking() {
        let mut manager = DelegateManager::new(3, 5);
        let root = Uuid::new_v4();
        let child = Uuid::new_v4();

        manager.record_depth(root, 0);
        let info = PendingTaskInfo {
            child_session_id: child,
            parent_session_id: root,
            agent_type: AgentType::Explorer,
            description: "Search".to_string(),
            prompt: "Find X".to_string(),
        };

        let depth = manager.track_task(info).unwrap();
        assert_eq!(depth, 1);
        assert_eq!(manager.active_task_count(root), 1);

        let removed = manager.complete_task(child);
        assert!(removed.is_some());
        assert_eq!(manager.active_task_count(root), 0);
    }

    #[test]
    fn test_max_depth_exceeded() {
        let mut manager = DelegateManager::new(1, 5);
        let root = Uuid::new_v4();
        let child = Uuid::new_v4();
        let grandchild = Uuid::new_v4();

        manager.record_depth(root, 0);

        // First delegation (depth 0 -> 1): OK (max_depth is 1, child depth will be 1)
        let info = PendingTaskInfo {
            child_session_id: child,
            parent_session_id: root,
            agent_type: AgentType::Explorer,
            description: "d1".to_string(),
            prompt: "p1".to_string(),
        };
        assert!(manager.track_task(info).is_ok());

        // Second delegation (depth 1 -> 2): exceeds max_depth of 1
        manager.record_depth(child, 1);
        let info = PendingTaskInfo {
            child_session_id: grandchild,
            parent_session_id: child,
            agent_type: AgentType::Explorer,
            description: "d2".to_string(),
            prompt: "p2".to_string(),
        };
        let result = manager.track_task(info);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Max delegation depth"));
    }

    #[test]
    fn test_max_pending_tasks() {
        let mut manager = DelegateManager::new(3, 2);
        let root = Uuid::new_v4();

        manager.record_depth(root, 0);

        // Create 2 tasks (max)
        for i in 0..2 {
            let info = PendingTaskInfo {
                child_session_id: Uuid::new_v4(),
                parent_session_id: root,
                agent_type: AgentType::Explorer,
                description: format!("d{}", i),
                prompt: "p".to_string(),
            };
            assert!(manager.track_task(info).is_ok());
        }

        // Third one should fail
        let info = PendingTaskInfo {
            child_session_id: Uuid::new_v4(),
            parent_session_id: root,
            agent_type: AgentType::Explorer,
            description: "d2".to_string(),
            prompt: "p".to_string(),
        };
        let result = manager.track_task(info);
        assert!(result.is_err());
    }
}
