use super::*;

pub(super) async fn list_mcp_servers(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<McpServerDto>> {
    let mcp = state.runtime.mcp_manager();
    let summaries = mcp.summaries();
    let all_tools = mcp.all_definitions();

    let mut dtos = Vec::new();
    for summary in summaries {
        let config = mcp.server_config(&summary.name);
        let is_disabled = summary.disabled || config.as_ref().is_some_and(|c| c.is_disabled());
        let (status_str, error_str) = match &summary.status {
            tidev_core::mcp::McpConnectionStatus::Connected => ("connected", None),
            tidev_core::mcp::McpConnectionStatus::Connecting => ("connecting", None),
            tidev_core::mcp::McpConnectionStatus::Disconnected => {
                if is_disabled {
                    ("disabled", None)
                } else {
                    ("disconnected", None)
                }
            }
            tidev_core::mcp::McpConnectionStatus::Failed(err) => ("failed", Some(err.clone())),
        };

        let tools = all_tools
            .iter()
            .filter(|tool| {
                tool.mcp_target()
                    .is_some_and(|(server, _)| server == summary.name)
            })
            .map(|tool| {
                let (_, raw_tool_name) = tool.mcp_target().unwrap_or(("", tool.name.as_str()));
                McpToolDto {
                    name: raw_tool_name.to_string(),
                    description: tool.description.clone(),
                    parameters: tool.parameters.clone(),
                }
            })
            .collect();

        dtos.push(McpServerDto {
            name: summary.name,
            kind: summary.kind,
            status: status_str.to_string(),
            error: error_str,
            disabled: is_disabled,
            config,
            tools,
        });
    }

    Json(dtos)
}

pub(super) async fn upsert_mcp_server(
    State(state): State<Arc<AppState>>,
    Json(request): Json<UpsertMcpServerRequest>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::bad_request("Server name cannot be empty"));
    }

    let mcp = state.runtime.mcp_manager().clone();
    if let Some(ref orig) = request.original_name {
        if orig != &name {
            let _ = mcp.remove_server(orig).await;
            state.runtime.update_config(|cfg| {
                cfg.mcp.servers.remove(orig);
            });
        }
    }

    mcp.upsert_server(name.clone(), request.config.clone())
        .await
        .map_err(|e| ApiError::internal(format!("{e:#}")))?;

    state.runtime.update_config(|cfg| {
        cfg.mcp.servers.insert(name, request.config);
    });
    state.runtime.save_config()?;

    Ok(Json(AcceptedResponse { accepted: true }))
}

pub(super) async fn delete_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    state
        .runtime
        .mcp_manager()
        .remove_server(&name)
        .await
        .map_err(|e| ApiError::internal(format!("{e:#}")))?;

    state.runtime.update_config(|cfg| {
        cfg.mcp.servers.remove(&name);
    });
    state.runtime.save_config()?;

    Ok(Json(AcceptedResponse { accepted: true }))
}

pub(super) async fn connect_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    let mut target_config = None;
    state.runtime.update_config(|cfg| {
        if let Some(srv) = cfg.mcp.servers.get_mut(&name) {
            srv.set_disabled(false);
            target_config = Some(srv.clone());
        }
    });
    state.runtime.save_config()?;

    if let Some(config) = target_config {
        state
            .runtime
            .mcp_manager()
            .upsert_server(name.clone(), config)
            .await
            .map_err(|e| ApiError::internal(format!("{e:#}")))?;
    }

    state
        .runtime
        .mcp_manager()
        .refresh_server(&name)
        .await
        .map_err(|e| ApiError::internal(format!("{e:#}")))?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

pub(super) async fn disconnect_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    let mut target_config = None;
    state.runtime.update_config(|cfg| {
        if let Some(srv) = cfg.mcp.servers.get_mut(&name) {
            srv.set_disabled(true);
            target_config = Some(srv.clone());
        }
    });
    state.runtime.save_config()?;

    if let Some(config) = target_config {
        state
            .runtime
            .mcp_manager()
            .upsert_server(name.clone(), config)
            .await
            .map_err(|e| ApiError::internal(format!("{e:#}")))?;
    }

    state
        .runtime
        .mcp_manager()
        .disconnect_server(&name)
        .await
        .map_err(|e| ApiError::internal(format!("{e:#}")))?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

pub(super) async fn refresh_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<AcceptedResponse>, ApiError> {
    state
        .runtime
        .mcp_manager()
        .refresh_server(&name)
        .await
        .map_err(|e| ApiError::internal(format!("{e:#}")))?;
    Ok(Json(AcceptedResponse { accepted: true }))
}

pub(crate) fn configured_auth_token(state: &AppState) -> Option<String> {
    state
        .runtime
        .auth()
        .web
        .auth_token
        .filter(|token| !token.trim().is_empty())
}

pub(crate) fn request_auth_token(headers: &HeaderMap, uri: &Uri) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(ToOwned::to_owned)
        .or_else(|| {
            uri.query().and_then(|query| {
                url::form_urlencoded::parse(query.as_bytes())
                    .find_map(|(key, value)| (key == "token").then(|| value.into_owned()))
            })
        })
}
