use std::sync::Arc;

use axum::{Json, extract::{Path, State}};
use anyhow::anyhow;

use crate::{AppState, errors::serialize_err};

use super::EndpointResult;

pub async fn get_deployment_status(
    Path(deployment_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> EndpointResult {
    let operation_kind = state
        .redis_manager
        .get_operation_kind(&deployment_id)
        .map_err(serialize_err)?;

    if let Some(operation_kind) = operation_kind {
        let operation_state = state
            .redis_manager
            .get_operation_state(&deployment_id)
            .map_err(serialize_err)?
            .ok_or_else(|| {
                serialize_err(anyhow!(
                    "Операция с id '{deployment_id}' не найдена или срок хранения истек"
                ))
            })?;
        let function_name = state
            .redis_manager
            .get_operation_function_name(&deployment_id)
            .map_err(serialize_err)?;
        let function_deployed = if let Some(name) = function_name.as_ref() {
            let deployed = state.function_manager.deployed_functions.read().await;
            Some(deployed.contains_key(name))
        } else {
            None
        };

        if operation_state == "failed" {
            let error = state
                .redis_manager
                .get_operation_error(&deployment_id)
                .map_err(serialize_err)?;
            let logs = state
                .redis_manager
                .get_operation_logs(&deployment_id)
                .map_err(serialize_err)?;

            return Ok(Json(serde_json::json!({
                "kind": operation_kind,
                "state": operation_state,
                "functionName": function_name,
                "functionDeployed": function_deployed,
                "accepted": false,
                "error": error,
                "logs": logs
            })));
        }

        return Ok(Json(serde_json::json!({
            "kind": operation_kind,
            "state": operation_state,
            "functionName": function_name,
            "functionDeployed": function_deployed,
            "accepted": operation_state == "finished"
        })));
    }

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
            "kind": "deploy",
            "state": deployment_state,
            "accepted": false,
            "error": error,
            "logs": logs
        })));
    }

    Ok(Json(serde_json::json!({
        "kind": "deploy",
        "state": deployment_state,
        "accepted": deployment_state == "finished"
    })))
}