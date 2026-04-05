use std::sync::Arc;

use axum::{Json, extract::{Path, State}};
use anyhow::anyhow;

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
    let deployment_state = deployment_state.ok_or_else(|| {
        serialize_err(anyhow!(
            "Деплой с id '{deployment_id}' не найден или срок хранения истек"
        ))
    })?;

    if deployment_state == "failed" {
        let error = state
            .redis_manager
            .get_deployment_error(&deployment_id)
            .map_err(serialize_err)?;
        let logs = state
            .redis_manager
            .get_deployment_logs(&deployment_id)
            .map_err(serialize_err)?;

        return Ok(Json(serde_json::json!({
            "state": deployment_state,
            "error": error,
            "logs": logs
        })));
    }

    Ok(Json(serde_json::json!({
        "state": deployment_state
    })))
}