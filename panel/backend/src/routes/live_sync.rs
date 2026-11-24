use axum::{
    extract::{Path, State},
    Json,
};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::ApiError;
use crate::services::live_sync::{
    check_preflight, execute_pull, execute_push, PreflightCheckRequest, PreflightCheckResponse,
    SyncRequest, SyncResponse,
};
use crate::AppState;

/// POST /api/sites/{id}/sync/preflight — Preflight connectivity check to remote panel.
pub async fn preflight(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<PreflightCheckRequest>,
) -> Result<Json<PreflightCheckResponse>, ApiError> {
    let local_domain = crate::helpers::site_domain_for_caller(&state, id, &claims).await?;
    let resp = check_preflight(&body, Some(&local_domain)).await;
    Ok(Json(resp))
}

/// POST /api/sites/{id}/sync/pull — Pull database and/or files from live server.
pub async fn pull(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SyncRequest>,
) -> Result<Json<SyncResponse>, ApiError> {
    let resp = execute_pull(&state, id, &body, &claims).await?;
    Ok(Json(resp))
}

/// POST /api/sites/{id}/sync/push — Push database and/or files to live server.
pub async fn push(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SyncRequest>,
) -> Result<Json<SyncResponse>, ApiError> {
    let resp = execute_push(&state, id, &body, &claims).await?;
    Ok(Json(resp))
}
