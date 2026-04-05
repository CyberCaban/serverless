use std::sync::Arc;

use axum::{Json, extract::{Path, State}};
use log::info;

use crate::{AppState, errors::serialize_err, redis_manager};

use super::EndpointResult;

pub async fn deploy_function(
    Path(function_name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> EndpointResult {
    let deployment_id = uuid::Uuid::now_v7().simple();
    info!(
        "Deploying function: '{}' with id: '{}'",
        function_name, &deployment_id
    );
    state
        .redis_manager
        .set_deployment_state(
            &deployment_id.to_string(),
            redis_manager::DeploymentState::Running,
        )
        .map_err(serialize_err)?;

    tokio::task::spawn(async move {
        let result = state
            .function_manager
            .redeploy_function_by_name(&function_name, &state.redis_manager)
            .await;
        match result {
            Ok(_) => {
                let _ = state.redis_manager.set_deployment_state(
                    &deployment_id.to_string(),
                    redis_manager::DeploymentState::Finished,
                );
            }
            Err(e) => {
                let _ = state.redis_manager.set_deployment_state(
                    &deployment_id.to_string(),
                    redis_manager::DeploymentState::Failed,
                );
                let _ = state
                    .redis_manager
                    .set_deployment_error(&deployment_id.to_string(), &e.to_string());
                let _ = state
                    .redis_manager
                    .append_deploy_logs(&deployment_id.to_string(), &e.to_string());
            }
        }
    });

    Ok(Json(serde_json::json!({
        "id": deployment_id
    })))
}