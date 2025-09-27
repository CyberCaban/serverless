use std::{fmt::format, fs, str::FromStr, sync::Arc};

use anyhow::{Ok, Result, anyhow, bail};
use axum::{
    Router,
    extract::{Path, State},
    response::Json,
    routing::post,
};
use bollard::{
    Docker, body_full,
    query_parameters::{BuildImageOptionsBuilder, CreateImageOptionsBuilder},
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncReadExt;

mod container_manager;
mod errors;
mod models;

use crate::errors::serialize_err;

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

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct FunctionConfig {
    name: String,
    #[serde(rename(serialize = "innerPort", deserialize = "innerPort"))]
    inner_port: u16,
    memory: u32,
    timeout: u32,
    version: String,
    dockerfile: String,
    #[serde(skip_deserializing, skip_serializing)]
    pub build_context_path: std::path::PathBuf,
}
impl FunctionConfig {
    pub async fn from_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        let content = tokio::fs::read_to_string(&path).await?;
        let mut config: FunctionConfig = serde_json::from_str(&content)?;
        config.build_context_path = path.as_ref().parent().unwrap().to_path_buf();
        Ok(config)
    }
}
struct ContainerManager {
    docker: Docker,
}
impl ContainerManager {
    pub fn new() -> Result<Self> {
        let docker = Docker::connect_with_local_defaults()?;
        Ok(Self { docker })
    }
    pub async fn build_image(
        &self,
        context_path: &str,
        image_name: &str,
        dockerfile_path: &str,
    ) -> Result<()> {
        let tar_path = self
            .create_build_context(context_path)
            .await
            .map_err(|e| anyhow!("Failed to create_build_context: {}", e))?;
        let build_image_options = BuildImageOptionsBuilder::new()
            .dockerfile(dockerfile_path)
            .t(image_name)
            .rm(true)
            .build();
        let bytes = Self::tar_to_bytes(&tar_path).await?;
        let mut image =
            self.docker
                .build_image(build_image_options, None, Some(body_full(bytes.into())));
        while let Some(info) = image.next().await {
            if let Err(e) = info {
                bail!("Build failed: {}", e)
            }
        }
        Ok(())
    }
    async fn tar_to_bytes(tar_path: &str) -> Result<Vec<u8>> {
        let mut file = tokio::fs::File::open(tar_path).await?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).await?;
        Ok(bytes)
    }
    async fn create_build_context(&self, path: &str) -> Result<String> {
        let tar_path = format!("/tmp/build-context-{}.tar", uuid::Uuid::new_v4());
        let output = tokio::process::Command::new("tar")
            .arg("-cf")
            .arg(&tar_path)
            .arg("-C")
            .arg(path)
            .arg(".")
            .output()
            .await
            .map_err(|e| anyhow!("Failed to create tar for build context: {}", e))?;
        if !output.status.success() {
            bail!(
                "Tar command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(tar_path)
    }
}
struct FunctionManager {
    container_manager: ContainerManager,
}
impl FunctionManager {
    pub fn new() -> Result<Self> {
        let container_manager = ContainerManager::new()?;
        Ok(Self { container_manager })
    }
    pub async fn read_function_config(path: &str) -> Result<FunctionConfig> {
        let path = format!("functions/{}/function.json", path);
        Ok(FunctionConfig::from_file(path).await?)
    }
    pub async fn build_function_image(&self, config: &FunctionConfig) -> Result<String> {
        let image_name = format!("{}:{}", config.name, config.version);
        self.container_manager
            .build_image(
                &config.build_context_path.to_string_lossy(),
                &image_name,
                &config.dockerfile,
            )
            .await?;
        Ok(image_name)
    }
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
