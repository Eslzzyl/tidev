use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct UndoConfirmDialogState {
    /// 选中的消息ID
    pub selected_message_id: Uuid,
    /// 消息内容预览
    pub message_content: String,
}

impl UndoConfirmDialogState {
    pub fn new(selected_message_id: Uuid, message_content: String) -> Self {
        Self {
            selected_message_id,
            message_content,
        }
    }

    pub fn title(&self) -> String {
        "Undo to message".to_string()
    }

    pub fn description(&self) -> String {
        let preview = if self.message_content.len() > 50 {
            format!("{}...", &self.message_content[..50])
        } else {
            self.message_content.clone()
        };
        format!(
            "Revert workspace to this message?\n\"{}\"\n\nThis will undo all changes after this message.",
            preview
        )
    }
}
