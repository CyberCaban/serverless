use std::{fs, sync::Arc};

use anyhow::{Ok, Result, anyhow, bail};
use axum::{
    Router,
    extract::{Path, State},
    response::Json,
    routing::post,
};
use serde_json::Value;

mod container_manager;
mod errors;
mod function_manager;
mod models;

use crate::{
    container_manager::ContainerManager, errors::serialize_err, function_manager::FunctionManager,
};

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
    let app = Router::new()
        .route("/deploy/{function_name}", post(deploy_function))
        .route("/invoke/{function_name}", post(invoke_function))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:5000").await?;
    println!("Running on port 5000");
    axum::serve(listener, app).await?;
    Ok(())
}

type EndpointResult = std::result::Result<Json<Value>, Json<Value>>;
async fn deploy_function(
    Path(function_name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> EndpointResult {
    let conf = FunctionManager::read_function_config(&function_name)
        .await
        .map_err(serialize_err)?;
    dbg!(&conf);
    tokio::task::spawn(async move { state.function_manager.build_function_image(&conf).await });
    std::result::Result::Ok(Json(serde_json::json!({
        "status": format!("Deploying {}...", function_name)
    })))
}

async fn invoke_function(function_name: String) -> Json<Value> {
    Json(serde_json::json!({
        "status": format!("Invoking {}...", function_name)
    }))
}
