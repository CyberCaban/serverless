use std::sync::Arc;

use axum::{Json, extract::{Path, State}};
use serde_json::Value;

use crate::{AppState, errors::serialize_err};

use super::EndpointResult;

pub async fn invoke_function(
    Path(function_name): Path<String>,
    State(state): State<Arc<AppState>>,
    payload: Option<Json<Value>>,
) -> EndpointResult {
    let payload_value = payload
        .map(|json| json.0)
        .unwrap_or_else(|| serde_json::json!({ "name": "test" }));

    let result = state
        .function_manager
        .try_invoke_with_meta(&function_name, payload_value)
        .await
        .map_err(serialize_err)?;

    Ok(Json(serde_json::json!({
        "function": function_name,
        "containerId": result.container_id,
        "result": result.result
    })))
}