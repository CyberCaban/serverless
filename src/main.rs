use crate::{
    container_manager::MANAGED_CONTAINER_LABEL,
    function_manager::FunctionManager, logger::setup_logger, redis_manager::RedisManager,
    routes::{
        deploy::deploy_function, get_status::get_deployment_status,
        invoke::invoke_function, list_functions::list_functions,
        replicas::get_function_replicas, stop::stop_function,
        update_config::update_function_config,
    },
    shutdown::shutdown_signal,
};
use anyhow::{Context, Result};
use axum::{Router, routing::{get, patch, post}};
use log::{info, warn};
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

fn cleanup_managed_containers_sync() -> Result<()> {
    let output = std::process::Command::new("docker")
        .args(["ps", "-aq", "--filter", "label=serverless.managed=true"])
        .output()
        .context("Failed to list managed containers")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !stderr.is_empty() {
            return Err(anyhow::anyhow!(stderr));
        }
        return Err(anyhow::anyhow!("docker ps failed"));
    }

    let ids = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if ids.is_empty() {
        return Ok(());
    }

    let status = std::process::Command::new("docker")
        .arg("rm")
        .arg("-f")
        .args(ids)
        .status()
        .context("Failed to remove managed containers")?;

    if !status.success() {
        return Err(anyhow::anyhow!("docker rm -f failed"));
    }

    Ok(())
}

fn install_panic_cleanup_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        if let Err(err) = cleanup_managed_containers_sync() {
            eprintln!("panic cleanup failed for containers ({MANAGED_CONTAINER_LABEL}): {err}");
        }
        default_hook(panic_info);
    }));
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
    install_panic_cleanup_hook();

    if let Err(err) = cleanup_managed_containers_sync() {
        warn!(
            "Startup cleanup failed for containers ({}): {}",
            MANAGED_CONTAINER_LABEL, err
        );
    }

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
        .route("/functions/{function_name}", patch(update_function_config))
        .route("/functions/{function_name}/stop", post(stop_function))
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
