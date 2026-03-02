use std::{collections::HashMap, sync::Arc};

use axum::{Json, extract::State};

use crate::AppState;

use super::EndpointResult;

pub async fn list_functions(State(state): State<Arc<AppState>>) -> EndpointResult {
    let guard_map = {
        let guard = state.function_manager.deployed_functions.read().await;
        serde_json::to_value(&*guard).map_err(|_| Json(serde_json::json!("failed to serialize")))?
    };
    let m = HashMap::from([("functions", guard_map)]);
    Ok(Json(serde_json::json!(m)))
}