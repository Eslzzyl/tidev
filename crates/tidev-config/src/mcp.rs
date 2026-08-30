use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::path::Path;

use crate::paths::ConfigPaths;

/// Top-level schema for standard `mcp.json` files.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpConfigFile {
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
}

/// In-memory MCP server configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpConfig {
    pub servers: BTreeMap<String, McpServerConfig>,
}

impl McpConfig {
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    /// Load the global `~/.config/tidev/mcp.json` file. If the file does not exist,
    /// returns an empty config.
    pub fn load(paths: &ConfigPaths) -> Result<Self> {
        Self::load_from_file(&paths.mcp_file)
    }

    /// Load MCP servers from a specific JSON file path.
    pub fn load_from_file(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read MCP config at {}", path.display()))?;
        let parsed: McpConfigFile = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse MCP config at {}", path.display()))?;
        Ok(Self {
            servers: parsed.mcp_servers,
        })
    }

    /// Load global MCP configuration and overlay project-level `<workspace_root>/.tidev/mcp.json`.
    pub fn load_with_workspace(paths: &ConfigPaths, workspace_root: &Path) -> Result<Self> {
        let mut config = Self::load(paths)?;
        let workspace_file = ConfigPaths::workspace_mcp_file(workspace_root);
        if workspace_file.exists() {
            let workspace_config = Self::load_from_file(&workspace_file)?;
            config.merge(workspace_config);
        }
        Ok(config)
    }

    /// Merge another MCP config into this one (overwriting servers with matching names).
    pub fn merge(&mut self, other: Self) {
        self.servers.extend(other.servers);
    }

    /// Save the configuration to the default global `mcp.json` path.
    pub fn save(&self, paths: &ConfigPaths) -> Result<()> {
        paths.ensure_directories()?;
        self.save_to_file(&paths.mcp_file)
    }

    /// Save the configuration to a specific JSON file path.
    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        let file_wrapper = McpConfigFile {
            mcp_servers: self.servers.clone(),
        };
        let contents = serde_json::to_string_pretty(&file_wrapper)
            .context("failed to serialize MCP config")?;
        std::fs::write(path, contents)
            .with_context(|| format!("failed to write MCP config to {}", path.display()))?;
        Ok(())
    }
}

/// Configuration for a single MCP server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpServerConfig {
    /// A server launched as a subprocess communicating over stdio.
    Stdio {
        command: String,
        args: Vec<String>,
        cwd: Option<String>,
        env: BTreeMap<String, String>,
    },
    /// A server accessible via HTTP POST.
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
    /// A legacy SSE server using a GET stream and a separate POST message endpoint.
    Sse {
        url: String,
        headers: BTreeMap<String, String>,
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

#[derive(Deserialize)]
struct RawMcpServerConfig {
    #[serde(default, alias = "kind")]
    r#type: Option<String>,
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    cwd: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    url: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

impl<'de> Deserialize<'de> for McpServerConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawMcpServerConfig::deserialize(deserializer)?;
        let kind = raw.r#type.as_deref().map(str::to_ascii_lowercase);

        match kind.as_deref() {
            Some("stdio") => {
                let command = raw
                    .command
                    .ok_or_else(|| serde::de::Error::missing_field("command"))?;
                Ok(Self::Stdio {
                    command,
                    args: raw.args,
                    cwd: raw.cwd,
                    env: raw.env,
                })
            }
            Some("sse") => {
                let url = raw
                    .url
                    .ok_or_else(|| serde::de::Error::missing_field("url"))?;
                Ok(Self::Sse {
                    url,
                    headers: raw.headers,
                })
            }
            Some("http") => {
                let url = raw
                    .url
                    .ok_or_else(|| serde::de::Error::missing_field("url"))?;
                Ok(Self::Http {
                    url,
                    headers: raw.headers,
                })
            }
            Some(other) => Err(serde::de::Error::custom(format!(
                "unknown MCP server type '{other}'"
            ))),
            None => {
                if let Some(command) = raw.command {
                    Ok(Self::Stdio {
                        command,
                        args: raw.args,
                        cwd: raw.cwd,
                        env: raw.env,
                    })
                } else if let Some(url) = raw.url {
                    Ok(Self::Http {
                        url,
                        headers: raw.headers,
                    })
                } else {
                    Err(serde::de::Error::custom(
                        "MCP server configuration must contain either 'command' or 'url'",
                    ))
                }
            }
        }
    }
}

impl Serialize for McpServerConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct StdioHelper<'a> {
            command: &'a str,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            args: &'a Vec<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            cwd: Option<&'a str>,
            #[serde(skip_serializing_if = "BTreeMap::is_empty")]
            env: &'a BTreeMap<String, String>,
        }

        #[derive(Serialize)]
        struct HttpHelper<'a> {
            url: &'a str,
            #[serde(skip_serializing_if = "BTreeMap::is_empty")]
            headers: &'a BTreeMap<String, String>,
        }

        #[derive(Serialize)]
        struct SseHelper<'a> {
            r#type: &'static str,
            url: &'a str,
            #[serde(skip_serializing_if = "BTreeMap::is_empty")]
            headers: &'a BTreeMap<String, String>,
        }

        match self {
            Self::Stdio {
                command,
                args,
                cwd,
                env,
            } => StdioHelper {
                command,
                args,
                cwd: cwd.as_deref(),
                env,
            }
            .serialize(serializer),
            Self::Http { url, headers } => HttpHelper { url, headers }.serialize(serializer),
            Self::Sse { url, headers } => SseHelper {
                r#type: "sse",
                url,
                headers,
            }
            .serialize(serializer),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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
    fn test_mcp_server_config_stdio_json_roundtrip() {
        let config = McpServerConfig::Stdio {
            command: "python".into(),
            args: vec!["-m".into(), "mcp_server".into()],
            cwd: Some("/project".into()),
            env: BTreeMap::from([("KEY".into(), "val".into())]),
        };

        let json_str = serde_json::to_string_pretty(&config).unwrap();
        let parsed: McpServerConfig = serde_json::from_str(&json_str).unwrap();

        match parsed {
            McpServerConfig::Stdio {
                command,
                args,
                cwd,
                env,
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
    fn test_mcp_server_config_http_json_roundtrip() {
        let config = McpServerConfig::Http {
            url: "https://example.com/mcp".into(),
            headers: BTreeMap::from([("Authorization".into(), "Bearer token".into())]),
        };

        let json_str = serde_json::to_string_pretty(&config).unwrap();
        let parsed: McpServerConfig = serde_json::from_str(&json_str).unwrap();

        match parsed {
            McpServerConfig::Http { url, headers } => {
                assert_eq!(url, "https://example.com/mcp");
                assert_eq!(headers.get("Authorization").unwrap(), "Bearer token");
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn test_mcp_server_config_sse_json_roundtrip() {
        let config = McpServerConfig::Sse {
            url: "http://localhost:8080/sse".into(),
            headers: BTreeMap::new(),
        };

        let json_str = serde_json::to_string_pretty(&config).unwrap();
        let parsed: McpServerConfig = serde_json::from_str(&json_str).unwrap();

        match parsed {
            McpServerConfig::Sse { url, headers } => {
                assert_eq!(url, "http://localhost:8080/sse");
                assert!(headers.is_empty());
            }
            other => panic!("expected Sse, got {other:?}"),
        }
    }

    #[test]
    fn test_mcp_config_file_json_parse_claude_desktop_format() {
        let json = r#"{
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path"]
                },
                "remote": {
                    "url": "https://api.example.com/mcp"
                },
                "events": {
                    "type": "sse",
                    "url": "https://api.example.com/sse",
                    "headers": {
                        "X-Token": "secret"
                    }
                }
            }
        }"#;

        let parsed: McpConfigFile = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.mcp_servers.len(), 3);
        assert!(matches!(
            parsed.mcp_servers.get("filesystem").unwrap(),
            McpServerConfig::Stdio { command, .. } if command == "npx"
        ));
        assert!(matches!(
            parsed.mcp_servers.get("remote").unwrap(),
            McpServerConfig::Http { url, .. } if url == "https://api.example.com/mcp"
        ));
        assert!(matches!(
            parsed.mcp_servers.get("events").unwrap(),
            McpServerConfig::Sse { url, headers } if url == "https://api.example.com/sse" && headers.get("X-Token") == Some(&"secret".to_string())
        ));
    }

    #[test]
    fn test_mcp_config_file_save_and_load() {
        let temp = tempdir().unwrap();
        let mcp_path = temp.path().join("mcp.json");

        let mut config = McpConfig::default();
        config.servers.insert(
            "test-server".into(),
            McpServerConfig::Stdio {
                command: "echo".into(),
                args: vec!["hello".into()],
                cwd: None,
                env: BTreeMap::new(),
            },
        );

        config.save_to_file(&mcp_path).unwrap();
        let loaded = McpConfig::load_from_file(&mcp_path).unwrap();
        assert_eq!(config, loaded);
    }

    #[test]
    fn test_mcp_config_workspace_overlay() {
        let temp = tempdir().unwrap();
        let global_dir = temp.path().join("global");
        let workspace_dir = temp.path().join("workspace");

        std::fs::create_dir_all(&global_dir).unwrap();
        std::fs::create_dir_all(workspace_dir.join(".tidev")).unwrap();

        let global_paths = ConfigPaths {
            config_dir: global_dir.clone(),
            data_dir: temp.path().join("data"),
            config_file: global_dir.join("config.toml"),
            mcp_file: global_dir.join("mcp.json"),
            auth_file: global_dir.join("auth.json"),
            database_file: global_dir.join("db.sqlite3"),
        };

        let mut global_config = McpConfig::default();
        global_config.servers.insert(
            "srv1".into(),
            McpServerConfig::Stdio {
                command: "cmd1".into(),
                args: vec![],
                cwd: None,
                env: BTreeMap::new(),
            },
        );
        global_config.servers.insert(
            "srv2".into(),
            McpServerConfig::Stdio {
                command: "cmd2_global".into(),
                args: vec![],
                cwd: None,
                env: BTreeMap::new(),
            },
        );
        global_config.save(&global_paths).unwrap();

        let mut workspace_config = McpConfig::default();
        workspace_config.servers.insert(
            "srv2".into(),
            McpServerConfig::Stdio {
                command: "cmd2_workspace".into(),
                args: vec![],
                cwd: None,
                env: BTreeMap::new(),
            },
        );
        workspace_config.servers.insert(
            "srv3".into(),
            McpServerConfig::Http {
                url: "http://example.com".into(),
                headers: BTreeMap::new(),
            },
        );
        workspace_config
            .save_to_file(&ConfigPaths::workspace_mcp_file(&workspace_dir))
            .unwrap();

        let merged = McpConfig::load_with_workspace(&global_paths, &workspace_dir).unwrap();
        assert_eq!(merged.servers.len(), 3);
        assert_eq!(merged.servers["srv1"].kind_label(), "stdio");
        match &merged.servers["srv2"] {
            McpServerConfig::Stdio { command, .. } => assert_eq!(command, "cmd2_workspace"),
            _ => panic!("unexpected kind"),
        }
        assert_eq!(merged.servers["srv3"].kind_label(), "http");
    }
}
