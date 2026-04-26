use crate::balance::DeepSeekBalanceResponse;

#[derive(Clone, Debug)]
pub struct BalancePanelState {
    pub active: bool,
    pub selected_provider: ProviderTab,
    pub deepseek_balance: Option<DeepSeekBalanceResponse>,
    pub loading: bool,
    pub error: Option<String>,
}

impl Default for BalancePanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl BalancePanelState {
    pub fn new() -> Self {
        Self {
            active: false,
            selected_provider: ProviderTab::DeepSeek,
            deepseek_balance: None,
            loading: false,
            error: None,
        }
    }

    pub fn toggle(&mut self) {
        self.active = !self.active;
        if !self.active {
            self.loading = false;
            self.error = None;
        }
    }

    pub fn open(&mut self) {
        self.active = true;
    }

    pub fn close(&mut self) {
        self.active = false;
        self.loading = false;
        self.error = None;
    }

    pub fn next_provider(&mut self) {
        self.selected_provider = match self.selected_provider {
            ProviderTab::DeepSeek => ProviderTab::DeepSeek,
        };
    }

    pub fn prev_provider(&mut self) {
        self.selected_provider = ProviderTab::DeepSeek;
    }

    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        if loading {
            self.error = None;
        }
    }

    pub fn set_balance(&mut self, balance: DeepSeekBalanceResponse) {
        self.deepseek_balance = Some(balance);
        self.loading = false;
        self.error = None;
    }

    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
        self.loading = false;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderTab {
    DeepSeek,
}

impl ProviderTab {
    pub fn label(&self) -> &'static str {
        match self {
            Self::DeepSeek => "DeepSeek",
        }
    }

    pub fn all() -> [Self; 1] {
        [Self::DeepSeek]
    }
}