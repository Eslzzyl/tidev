use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// MCP server configuration section in `config.toml`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: BTreeMap<String, McpServerConfig>,
}

impl McpConfig {
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }
}

/// Configuration for a single MCP server.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpServerConfig {
    /// A server launched as a subprocess communicating over stdio.
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    /// A server accessible via HTTP POST.
    Http {
        url: String,
    },
    /// A server accessible via Server-Sent Events.
    Sse {
        url: String,
    },
}

impl McpServerConfig {
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Stdio { .. } => "stdio",
            Self::Http { .. } => "http",
            Self::Sse { .. } => "sse",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_config_default_is_empty() {
        let cfg = McpConfig::default();
        assert!(cfg.is_empty());
    }

    #[test]
    fn test_mcp_config_not_empty() {
        let mut cfg = McpConfig::default();
        cfg.servers.insert(
            "my-server".into(),
            McpServerConfig::Stdio {
                command: "node".into(),
                args: vec!["server.js".into()],
                cwd: None,
                env: BTreeMap::new(),
            },
        );
        assert!(!cfg.is_empty());
    }

    #[test]
    fn test_mcp_server_config_stdio_roundtrip() {
        let config = McpServerConfig::Stdio {
            command: "python".into(),
            args: vec!["-m".into(), "mcp_server".into()],
            cwd: Some("/project".into()),
            env: BTreeMap::from([("KEY".into(), "val".into())]),
        };

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: McpServerConfig = toml::from_str(&toml_str).unwrap();

        match parsed {
            McpServerConfig::Stdio {
                command, args, cwd, env, ..
            } => {
                assert_eq!(command, "python");
                assert_eq!(args, vec!["-m", "mcp_server"]);
                assert_eq!(cwd, Some("/project".into()));
                assert_eq!(env.get("KEY"), Some(&"val".into()));
            }
            other => panic!("expected Stdio, got {other:?}"),
        }
    }

    #[test]
    fn test_mcp_server_config_http_roundtrip() {
        let config = McpServerConfig::Http {
            url: "https://example.com/mcp".into(),
        };

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: McpServerConfig = toml::from_str(&toml_str).unwrap();

        match parsed {
            McpServerConfig::Http { url } => {
                assert_eq!(url, "https://example.com/mcp");
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn test_mcp_server_config_sse_roundtrip() {
        let config = McpServerConfig::Sse {
            url: "http://localhost:8080/sse".into(),
        };

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: McpServerConfig = toml::from_str(&toml_str).unwrap();

        match parsed {
            McpServerConfig::Sse { url } => {
                assert_eq!(url, "http://localhost:8080/sse");
            }
            other => panic!("expected Sse, got {other:?}"),
        }
    }

    #[test]
    fn test_mcp_server_config_kind_label() {
        let stdio = McpServerConfig::Stdio {
            command: "cmd".into(),
            args: vec![],
            cwd: None,
            env: BTreeMap::new(),
        };
        let http = McpServerConfig::Http {
            url: "http://e.com".into(),
        };
        let sse = McpServerConfig::Sse {
            url: "http://e.com/sse".into(),
        };

        assert_eq!(stdio.kind_label(), "stdio");
        assert_eq!(http.kind_label(), "http");
        assert_eq!(sse.kind_label(), "sse");
    }

    #[test]
    fn test_mcp_server_config_stdio_default_args() {
        // Verify that args defaults to empty in TOML deserialization.
        let toml = r#"
kind = "stdio"
command = "node"
"#;
        let config: McpServerConfig = toml::from_str(toml).unwrap();
        match config {
            McpServerConfig::Stdio { args, .. } => {
                assert!(args.is_empty());
            }
            other => panic!("expected Stdio, got {other:?}"),
        }
    }
}
