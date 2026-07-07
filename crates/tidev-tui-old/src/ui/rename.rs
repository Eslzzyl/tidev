#[derive(Clone, Debug)]
pub struct RenameSessionDialogState {
    pub original_title: String,
}

impl RenameSessionDialogState {
    pub fn new(original_title: String) -> Self {
        Self { original_title }
    }

    pub fn title(&self) -> String {
        "Rename session".to_string()
    }

    pub fn description(&self) -> String {
        format!("Current title: {}", self.original_title)
    }
}
