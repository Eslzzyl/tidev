use axum::{Json, extract::State};
use serde::Serialize;

use crate::tooling::SkillCatalog;
use crate::web::error::WebResult;
use crate::web::state::AppState;

/// Skill info for the API response
#[derive(Serialize)]
pub struct SkillInfoResponse {
    pub name: String,
    pub description: String,
    pub location: String,
}

/// List skills response
#[derive(Serialize)]
pub struct ListSkillsResponse {
    pub skills: Vec<SkillInfoResponse>,
}

/// List all available skills
pub async fn list_skills(State(state): State<AppState>) -> WebResult<Json<ListSkillsResponse>> {
    log::debug!("Listing available skills");

    let config = state.config.read().await;
    let skill_sources = config.skills.clone();
    drop(config);

    let catalog = SkillCatalog::discover(
        &state.workspace_root,
        &state.config_dir,
        &skill_sources,
        None,
    );

    let skills: Vec<SkillInfoResponse> = catalog
        .all()
        .iter()
        .map(|s| SkillInfoResponse {
            name: s.name.clone(),
            description: s.description.clone(),
            location: s.location.display().to_string(),
        })
        .collect();

    log::info!("Listed {} skills", skills.len());
    Ok(Json(ListSkillsResponse { skills }))
}
