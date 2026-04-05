use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
};

use crate::{AppState, errors::serialize_err};

use super::EndpointResult;

pub async fn stop_function(
    Path(function_name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> EndpointResult {
    let removed = state
        .function_manager
        .stop_function(&function_name, &state.redis_manager)
        .await
        .map_err(serialize_err)?;

    Ok(Json(serde_json::json!({
        "function": function_name,
        "status": "stopped",
        "removedContainers": removed
    })))
}
