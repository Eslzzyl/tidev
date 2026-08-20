//! Workspace-scoped resources.
//!
//! A [`Workspace`] bundles all state that is tied to a specific project
//! directory: skills, MCP servers, tool registry, git service, file search
//! index and snapshot service.  The runtime owns a default workspace (the
//! directory it was started in) and lazily creates/cache additional workspaces
//! when frontends ask for them.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use anyhow::Result;

use tidev_config::{AppConfig, AuthStore, paths::ConfigPaths};
use tidev_search::FileSearchIndex;
use tidev_snapshot::SnapshotService;
use tidev_tools::{SkillCatalog, TodoPersistence};
use tidev_utils::path::canonicalize_display;

use crate::git::GitService;
use crate::mcp::McpManager;
use crate::registry::ToolRegistry;

/// All resources bound to a single workspace directory.
#[derive(Clone)]
pub struct Workspace {
    root: PathBuf,
    config: AppConfig,
    skills: SkillCatalog,
    mcp_manager: McpManager,
    git: GitService,
    snapshot: Option<SnapshotService>,
    tool_registry: Arc<ToolRegistry>,
    file_search_index: OnceLock<Arc<FileSearchIndex>>,
}

impl Workspace {
    /// Initialise a workspace for an existing directory.
    ///
    /// `config` is the effective configuration for this workspace (global
    /// config with the workspace-level `.tidev/config.toml` overlay already
    /// applied).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: PathBuf,
        paths: &ConfigPaths,
        config: &AppConfig,
        auth: &AuthStore,
        max_output_bytes: usize,
        todo: Arc<dyn TodoPersistence + Send + Sync + 'static>,
    ) -> Result<Self> {
        let root = canonicalize_display(&root);
        if !std::fs::metadata(&root)
            .map(|m| m.is_dir())
            .unwrap_or(false)
        {
            anyhow::bail!("workspace is not a directory: {}", root.display());
        }

        let skills = SkillCatalog::discover(&root, &paths.config_dir, &config.skills, None);
        let mcp_manager = McpManager::new(root.clone(), config.mcp.servers.clone());
        let tool_registry = Arc::new(ToolRegistry::new(
            root.clone(),
            paths.config_dir.clone(),
            skills.clone(),
            todo,
            config.websearch.clone(),
            auth.clone(),
            max_output_bytes,
            mcp_manager.clone(),
        ));

        let snapshot = if config.snapshot.enabled {
            Some(SnapshotService::new(
                &root,
                paths,
                Arc::new(config.snapshot.clone()),
            )?)
        } else {
            None
        };

        let git = GitService::new(root.clone());

        Ok(Self {
            root,
            config: config.clone(),
            skills,
            mcp_manager,
            git,
            snapshot,
            tool_registry,
            file_search_index: OnceLock::new(),
        })
    }

    /// Workspace root directory.
    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    /// Effective configuration for this workspace.
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    /// Skills catalog discovered in this workspace.
    pub fn skills(&self) -> &SkillCatalog {
        &self.skills
    }

    /// MCP manager for this workspace.
    pub fn mcp_manager(&self) -> &McpManager {
        &self.mcp_manager
    }

    /// Tool registry bound to this workspace.
    pub fn tool_registry(&self) -> &ToolRegistry {
        &self.tool_registry
    }

    /// Cloneable handle to the tool registry.
    pub fn tool_registry_arc(&self) -> Arc<ToolRegistry> {
        Arc::clone(&self.tool_registry)
    }

    /// Git service for this workspace.
    pub fn git(&self) -> GitService {
        self.git.clone()
    }

    /// Snapshot service for this workspace, if enabled.
    pub fn snapshot(&self) -> Option<&SnapshotService> {
        self.snapshot.as_ref()
    }

    /// Lazy file search index for this workspace.
    pub fn file_search_index(&self) -> Arc<FileSearchIndex> {
        self.file_search_index
            .get_or_init(|| {
                let index = Arc::new(FileSearchIndex::new());
                index.ensure_background_indexing(&self.root);
                index
            })
            .clone()
    }
}
