use crate::ContainerManager;
use anyhow::{Ok, Result};
use serde::Deserialize;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct FunctionConfig {
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

pub struct FunctionManager {
    container_manager: ContainerManager,
}

impl FunctionManager {
    pub fn new() -> Result<Self> {
        let container_manager = ContainerManager::new()?;
        Ok(Self { container_manager })
    }

    pub async fn read_function_config(path: &str) -> Result<FunctionConfig> {
        let path = format!("functions/{path}/function.json");
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
