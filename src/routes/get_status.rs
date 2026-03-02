use std::sync::Arc;

use axum::{Json, extract::{Path, State}};

use crate::{AppState, errors::serialize_err};

use super::EndpointResult;

pub async fn get_deployment_status(
    Path(deployment_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> EndpointResult {
    let deployment_state = state
        .redis_manager
        .get_deployment_state(&deployment_id)
        .map_err(serialize_err)?;
    Ok(Json(serde_json::json!({
        "state": deployment_state
    })))
}