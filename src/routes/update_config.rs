use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
};

use crate::{AppState, errors::serialize_err, function_manager::FunctionConfigUpdate};
use crate::redis_manager;

use super::EndpointResult;

pub async fn update_function_config(
    Path(function_name): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(update): Json<FunctionConfigUpdate>,
) -> EndpointResult {
    let operation_id = uuid::Uuid::now_v7().simple();
    let operation_id_string = operation_id.to_string();
    let function_name_for_task = function_name.clone();
    let operation_id_for_task = operation_id_string.clone();

    state
        .redis_manager
        .set_operation_kind(&operation_id_string, "config_update")
        .map_err(serialize_err)?;
    state
        .redis_manager
        .set_operation_function_name(&operation_id_string, &function_name)
        .map_err(serialize_err)?;
    state
        .redis_manager
        .set_operation_state(
            &operation_id_string,
            redis_manager::DeploymentState::Running,
        )
        .map_err(serialize_err)?;

    tokio::task::spawn(async move {
        let result = state
            .function_manager
            .update_function_config(&function_name_for_task, update, &state.redis_manager)
            .await;

        match result {
            Ok(_) => {
                let _ = state.redis_manager.set_operation_state(
                    &operation_id_for_task,
                    redis_manager::DeploymentState::Finished,
                );
            }
            Err(error) => {
                let _ = state.redis_manager.set_operation_state(
                    &operation_id_for_task,
                    redis_manager::DeploymentState::Failed,
                );
                let _ = state
                    .redis_manager
                    .set_operation_error(&operation_id_for_task, &error.to_string());
                let _ = state
                    .redis_manager
                    .append_operation_logs(&operation_id_for_task, &error.to_string());
            }
        }
    });

    Ok(Json(serde_json::json!({
        "function": function_name,
        "id": operation_id_string
    })))
}
