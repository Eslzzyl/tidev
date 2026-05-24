use tidev_engine::agent::AgentType;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct AgentInfo {
    pub agent_type: AgentType,
    pub display_name: String,
    pub description: String,
    pub read_only: bool,
    pub tools: Vec<String>,
    pub temperature: f32,
}

#[derive(Clone, Debug)]
pub struct AgentsPanelState {
    pub agents: Vec<AgentInfo>,
    pub scroll_offset: usize,
}

impl AgentsPanelState {
    pub fn new() -> Self {
        let agents = AgentType::all()
            .iter()
            .map(|at| {
                let tools = at
                    .default_tool_restrictions()
                    .map(|t| t.iter().map(|s| s.to_string()).collect())
                    .unwrap_or_else(|| vec!["all".to_string()]);
                AgentInfo {
                    agent_type: *at,
                    display_name: at.display_name().to_string(),
                    description: at.description().to_string(),
                    read_only: at.is_read_only(),
                    tools,
                    temperature: at.default_temperature(),
                }
            })
            .collect();

        Self {
            agents,
            scroll_offset: 0,
        }
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
    }
}
