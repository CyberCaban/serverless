use crate::{
    container_manager::ContainerManager, errors::serialize_err, function_manager::FunctionManager,
    logger::setup_logger, redis_manager::RedisManager, shutdown::shutdown_signal,
};
use anyhow::{Context, Result};
use axum::{
    Router,
    extract::{Path, State},
    response::Json,
    routing::{get, post},
};
use log::info;
use serde_json::{Value, json};
use std::{collections::HashMap, fs, sync::Arc};

extern crate redis;

mod container_manager;
mod deployed_functions;
mod errors;
mod function_manager;
mod logger;
mod models;
mod redis_manager;
mod shutdown;

struct Config {
    // Contains paths to directories in functions dir
    function_paths: Vec<String>,
}

fn read_function_paths() -> Vec<String> {
    fs::read_dir("functions")
        .expect("Missing functions directory")
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_file() {
                return None;
            }
            Some(path.display().to_string())
        })
        .collect::<Vec<String>>()
}

#[derive(Debug)]
struct AppState {
    function_manager: FunctionManager,
    redis_manager: RedisManager,
}
impl AppState {
    pub fn new() -> Result<Self> {
        let function_manager = FunctionManager::new()?;
        let redis_manager = RedisManager::new().context("Redis error")?;
        Ok(Self {
            function_manager,
            redis_manager,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_logger()?;

    let paths = read_function_paths();
    info!("Read function paths");
    let config = Config {
        function_paths: paths,
    };
    let state = {
        let state = AppState::new()?;
        Arc::new(state)
    };
    let cleanup_state = Arc::clone(&state);
    let port = 5000;
    let app = Router::new()
        .route("/deploy/{function_name}", post(deploy_function))
        .route("/deploy/status/{deployment_id}", get(get_deployment_status))
        .route("/invoke/{function_name}", post(invoke_function))
        .route("/functions", get(list_functions))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    println!("Running on port {port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(cleanup_state))
        .await?;
    Ok(())
}

type EndpointResult<T = Value> = std::result::Result<Json<T>, Json<T>>;
async fn deploy_function(
    Path(function_name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> EndpointResult {
    let conf = FunctionManager::read_function_config(&function_name)
        .await
        .map_err(serialize_err)?;
    // TODO add a way to not block execution and tell the errors if there any
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
        let result = state.function_manager.deploy_function(conf).await;
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
                    .append_deploy_logs(&deployment_id.to_string(), &e.to_string());
            }
        }
    });

    Ok(Json(serde_json::json!({
        "id": deployment_id
    })))
}

async fn list_functions(State(state): State<Arc<AppState>>) -> EndpointResult {
    let guard_map = {
        let guard = state.function_manager.deployed_functions.read().await;
        serde_json::to_value(&*guard).map_err(|_| Json(serde_json::json!("failed to serialize")))?
    };
    let m = HashMap::from([("functions", guard_map)]);
    Ok(Json(serde_json::json!(m)))
}

async fn get_deployment_status(
    Path(deployment_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> EndpointResult {
    let deployment_state = state
        .redis_manager
        .get_deployment_state(&deployment_id)
        .map_err(serialize_err)?;
    Ok(Json(json!({
        "state": deployment_state
    })))
}

async fn invoke_function(
    Path(function_name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> EndpointResult {
    state
        .function_manager
        .try_invoke(&function_name)
        .await
        .map_err(serialize_err)?;
    Ok(Json(serde_json::json!({
        "status": format!("Invoking {}...", function_name)
    })))
}
