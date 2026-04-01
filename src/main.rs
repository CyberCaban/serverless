use crate::{
    function_manager::FunctionManager, logger::setup_logger, redis_manager::RedisManager,
    routes::{
        deploy::deploy_function, get_status::get_deployment_status,
        invoke::invoke_function, list_functions::list_functions,
        replicas::get_function_replicas,
    },
    shutdown::shutdown_signal,
};
use anyhow::{Context, Result};
use axum::{Router, routing::{get, post}};
use log::info;
use std::{fs, sync::Arc};

extern crate redis;

mod container_manager;
mod deployed_functions;
mod errors;
mod function_manager;
mod logger;
mod balancers;
mod models;
mod redis_manager;
mod routes;
mod shutdown;

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

pub(crate) struct AppState {
    function_manager: FunctionManager,
    redis_manager: RedisManager,
}
impl AppState {
    pub async fn new() -> Result<Self> {
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
    info!("Discovered {} function directories", paths.len());
    let state = {
        let state = AppState::new().await?;
        Arc::new(state)
    };
    let cleanup_state = Arc::clone(&state);
    let port = 5000;
    let app = Router::new()
        .route("/deploy/{function_name}", post(deploy_function))
        .route("/deploy/status/{deployment_id}", get(get_deployment_status))
        .route("/invoke/{function_name}", post(invoke_function))
        .route("/functions/{function_name}/replicas", get(get_function_replicas))
        .route("/functions", get(list_functions))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    println!("Running on port {port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(cleanup_state))
        .await?;
    Ok(())
}
