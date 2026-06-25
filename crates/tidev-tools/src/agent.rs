/// Agent type classification for sub-agent delegation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentType {
    General,
    Explorer,
    Librarian,
    Oracle,
    Designer,
    Fixer,
}

impl AgentType {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "general" => Some(Self::General),
            "explorer" => Some(Self::Explorer),
            "librarian" => Some(Self::Librarian),
            "oracle" => Some(Self::Oracle),
            "designer" => Some(Self::Designer),
            "fixer" => Some(Self::Fixer),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Explorer => "explorer",
            Self::Librarian => "librarian",
            Self::Oracle => "oracle",
            Self::Designer => "designer",
            Self::Fixer => "fixer",
        }
    }
}
