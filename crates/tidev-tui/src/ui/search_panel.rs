use crate::input::Composer;
use tidev_config::auth::AuthStore;

/// Static metadata for a built-in search provider.
#[derive(Clone, Debug)]
pub struct SearchProviderInfo {
    pub id: &'static str,
    pub display_name: &'static str,
    pub needs_api_key: bool,
    pub needs_cx: bool,
    #[allow(dead_code)]
    pub description: &'static str,
}

/// All built-in providers.
pub const BUILTIN_PROVIDERS: &[SearchProviderInfo] = &[
    SearchProviderInfo {
        id: "exa",
        display_name: "Exa",
        needs_api_key: false,
        needs_cx: false,
        description: "Public endpoint, no key required",
    },
    SearchProviderInfo {
        id: "brave",
        display_name: "Brave Search",
        needs_api_key: true,
        needs_cx: false,
        description: "Free tier: 2,000 queries/month",
    },
    SearchProviderInfo {
        id: "google",
        display_name: "Google Custom Search",
        needs_api_key: true,
        needs_cx: true,
        description: "Free tier: 100 queries/day",
    },
    SearchProviderInfo {
        id: "tavily",
        display_name: "Tavily",
        needs_api_key: true,
        needs_cx: false,
        description: "Free tier: 1,000 requests/month",
    },
];

#[derive(Clone, Debug)]
pub struct SearchPanelState {
    pub selected_index: usize,
    pub active_provider: String,
    pub editing_api_key: Option<String>,
    pub input_buffer: Composer,
    pub editing_cx: bool,
}

impl SearchPanelState {
    pub fn new(active_provider: &str) -> Self {
        Self {
            selected_index: 0,
            active_provider: active_provider.to_string(),
            editing_api_key: None,
            input_buffer: Composer::new(""),
            editing_cx: false,
        }
    }

    pub fn provider_count(&self) -> usize {
        BUILTIN_PROVIDERS.len()
    }

    pub fn move_selection(&mut self, delta: isize) {
        let count = self.provider_count();
        if count == 0 {
            return;
        }
        let new = (self.selected_index as isize + delta).rem_euclid(count as isize) as usize;
        self.selected_index = new;
    }

    pub fn selected_provider_missing_key(&self, auth: &AuthStore) -> bool {
        self.selected_index < BUILTIN_PROVIDERS.len() && {
            let info = &BUILTIN_PROVIDERS[self.selected_index];
            info.needs_api_key && !auth.web.search_api_keys.contains_key(info.id)
        }
    }

    pub fn selected_provider_missing_cx(&self, auth: &AuthStore) -> bool {
        self.selected_index < BUILTIN_PROVIDERS.len() && {
            let info = &BUILTIN_PROVIDERS[self.selected_index];
            info.needs_cx && auth.web.google_cx.is_none()
        }
    }

    pub fn start_editing_api_key(&mut self) {
        if self.selected_index < BUILTIN_PROVIDERS.len() {
            let info = &BUILTIN_PROVIDERS[self.selected_index];
            let placeholder = format!("Enter API key for {}: ", info.display_name);
            self.editing_api_key = Some(info.id.to_string());
            self.editing_cx = false;
            self.input_buffer.clear();
            self.input_buffer.set_placeholder(&placeholder);
        }
    }

    pub fn start_editing_cx(&mut self) {
        self.editing_api_key = Some("google".to_string());
        self.editing_cx = true;
        self.input_buffer.clear();
        self.input_buffer
            .set_placeholder("Enter Google Search Engine ID (cx): ");
    }

    pub fn provider_status(&self, index: usize, auth: &AuthStore) -> String {
        if index >= BUILTIN_PROVIDERS.len() {
            return String::new();
        }
        let info = &BUILTIN_PROVIDERS[index];

        let status = if info.needs_cx {
            if auth.web.search_api_keys.contains_key(info.id) && auth.web.google_cx.is_some() {
                "Ready"
            } else if !auth.web.search_api_keys.contains_key(info.id) {
                "Set API key"
            } else {
                "Set Search Engine ID"
            }
        } else if info.needs_api_key {
            if auth.web.search_api_keys.contains_key(info.id) {
                "Ready"
            } else {
                "Set API key"
            }
        } else {
            "Ready"
        };

        format!("{}  —  {}", info.display_name, status)
    }
}
