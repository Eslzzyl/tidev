use axum::Json;
use serde::Serialize;

use crate::web::error::WebResult;

/// Tool info
#[derive(Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub permission: String,
}

/// List tools response
#[derive(Serialize)]
pub struct ListToolsResponse {
    pub tools: Vec<ToolInfo>,
}

/// List all available tools
pub async fn list_tools() -> WebResult<Json<ListToolsResponse>> {
    crate::log_debug!("Listing available tools");

    // For MVP, return static list of built-in tools
    let tools = vec![
        ToolInfo {
            name: "read".to_string(),
            display_name: "Read File".to_string(),
            description: "Read the contents of a file".to_string(),
            permission: "read".to_string(),
        },
        ToolInfo {
            name: "write".to_string(),
            display_name: "Write File".to_string(),
            description: "Write content to a file".to_string(),
            permission: "write".to_string(),
        },
        ToolInfo {
            name: "edit".to_string(),
            display_name: "Edit File".to_string(),
            description: "Edit a file by replacing text".to_string(),
            permission: "edit".to_string(),
        },
        ToolInfo {
            name: "bash".to_string(),
            display_name: "Execute Shell".to_string(),
            description: "Execute a shell command".to_string(),
            permission: "execute".to_string(),
        },
        ToolInfo {
            name: "list".to_string(),
            display_name: "List Directory".to_string(),
            description: "List files in a directory".to_string(),
            permission: "read".to_string(),
        },
        ToolInfo {
            name: "glob".to_string(),
            display_name: "Glob Search".to_string(),
            description: "Search files using glob patterns".to_string(),
            permission: "search".to_string(),
        },
        ToolInfo {
            name: "grep".to_string(),
            display_name: "Grep Search".to_string(),
            description: "Search file contents using regex".to_string(),
            permission: "search".to_string(),
        },
        ToolInfo {
            name: "websearch".to_string(),
            display_name: "Web Search".to_string(),
            description: "Search the web".to_string(),
            permission: "search".to_string(),
        },
        ToolInfo {
            name: "webfetch".to_string(),
            display_name: "Web Fetch".to_string(),
            description: "Fetch a web page".to_string(),
            permission: "search".to_string(),
        },
    ];

    crate::log_debug!("Listed {} tools", tools.len());
    Ok(Json(ListToolsResponse { tools }))
}
