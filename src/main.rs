use crate::{
    container_manager::ContainerManager, errors::serialize_err, function_manager::FunctionManager,
    shutdown::shutdown_signal,
};
use anyhow::Result;
use axum::{
    Router,
    extract::{Path, State},
    response::Json,
    routing::{get, post},
};
use serde_json::Value;
use std::{collections::HashMap, fs, sync::Arc};

mod container_manager;
mod deployed_functions;
mod errors;
mod function_manager;
mod models;
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
}
impl AppState {
    pub fn new() -> Result<Self> {
        let function_manager = FunctionManager::new()?;
        Ok(Self { function_manager })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let paths = read_function_paths();
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
    dbg!(&conf);
    // TODO add a way to not block execution and tell the errors if there any
    // tokio::task::spawn(async move { state.function_manager.build_function_image(&conf).await });
    state
        .function_manager
        .deploy_function(conf)
        .await
        .map_err(serialize_err)?;
    Ok(Json(serde_json::json!({
        "status": format!("Deploying {}...", function_name)
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
