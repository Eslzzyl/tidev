use super::{App, GitTaskResult};

use crate::action::{Action, GitAction, GitQueryKind};
use tidev_core::GitDiffScope;

impl App {
    fn next_git_request_id(&mut self) -> u64 {
        self.next_git_request_id = self.next_git_request_id.wrapping_add(1);
        self.next_git_request_id
    }

    pub(crate) fn spawn_git_status(&mut self) -> u64 {
        let request_id = self.next_git_request_id();
        let service = self.runtime.git();
        let tx = self.git_result_tx.clone();
        tokio::spawn(async move {
            let result = service.status().await;
            let _ = tx.send(GitTaskResult::Status { request_id, result });
        });
        request_id
    }

    pub(crate) fn spawn_git_history(&mut self, head: Option<String>, skip: usize) -> u64 {
        let request_id = self.next_git_request_id();
        let service = self.runtime.git();
        let tx = self.git_result_tx.clone();
        tokio::spawn(async move {
            let result = service.history(head.as_deref(), skip, 50).await;
            let _ = tx.send(GitTaskResult::History { request_id, result });
        });
        request_id
    }

    pub(crate) fn spawn_git_diff(&mut self, scope: GitDiffScope) -> u64 {
        let request_id = self.next_git_request_id();
        let service = self.runtime.git();
        let tx = self.git_result_tx.clone();
        tokio::spawn(async move {
            let result = service.diff(scope).await;
            let _ = tx.send(GitTaskResult::Diff { request_id, result });
        });
        request_id
    }

    pub(crate) fn handle_git_task_result(&mut self, result: GitTaskResult) {
        let action = match result {
            GitTaskResult::Status { request_id, result } => {
                GitAction::StatusReady { request_id, result }
            }
            GitTaskResult::History { request_id, result } => {
                GitAction::HistoryReady { request_id, result }
            }
            GitTaskResult::Diff { request_id, result } => {
                GitAction::DiffReady { request_id, result }
            }
        };
        self.process_action(Action::Git(action));
    }

    pub(crate) fn git_loading_action(request_id: u64, query: GitQueryKind) -> Action {
        Action::Git(GitAction::Loading { request_id, query })
    }
}
