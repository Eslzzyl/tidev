use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct ForkConfirmDialogState {
    /// 选中的消息ID
    pub selected_message_id: Uuid,
    /// 要复制的消息数量
    pub message_count: usize,
}

impl ForkConfirmDialogState {
    pub fn new(selected_message_id: Uuid, message_count: usize) -> Self {
        Self {
            selected_message_id,
            message_count,
        }
    }

    pub fn title(&self) -> String {
        "Fork session".to_string()
    }

    pub fn description(&self) -> String {
        format!(
            "Create a new session from this message? This will copy {} message{} to a new session.",
            self.message_count,
            if self.message_count == 1 { "" } else { "s" }
        )
    }
}
