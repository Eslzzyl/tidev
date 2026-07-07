//! [`TodoPersistence`] trait — abstraction for todo list storage.
//!
//! tidev-tools defines this trait so that tools (like `todowrite`) can
//! persist todo items without depending on `tidev-storage` directly.
//! tidev-core implements this trait by bridging to `SessionStore`.

use tidev_types::TodoItem;

/// Abstraction for persisting todo items scoped to a session.
///
/// This trait has only two methods, keeping the abstraction as lightweight
/// as possible while still cutting the dependency on `tidev-storage`.
pub trait TodoPersistence: Send + Sync {
    /// Load all todo items for the given session.
    fn load_todos(&self, session_id: uuid::Uuid) -> anyhow::Result<Vec<TodoItem>>;

    /// Atomically replace the todo list for the given session.
    fn replace_todos(
        &self,
        session_id: uuid::Uuid,
        todos: &[TodoItem],
    ) -> anyhow::Result<()>;
}
