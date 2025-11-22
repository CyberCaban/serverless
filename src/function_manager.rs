use crate::{
    ContainerManager, deployed_functions::DeployedFunctions, errors::function_error::FunctionError,
};
use anyhow::Result;
use bollard::secret::ContainerCreateBody;
use serde::{Deserialize, Serialize};
use std::result::Result::Ok;

#[allow(dead_code)]
#[derive(Debug, Deserialize, Serialize)]
pub struct FunctionConfig {
    pub name: String,
    #[serde(rename(serialize = "innerPort", deserialize = "innerPort"))]
    pub inner_port: u16,
    pub memory: i64,
    pub timeout: u32,
    pub version: String,
    pub dockerfile: String,
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

#[derive(Debug, Serialize)]
pub struct RunningFunction {
    pub config: FunctionConfig,
    pub container_config: ContainerCreateBody,
    pub container_ids: Vec<String>,
}

#[derive(Debug)]
pub struct FunctionManager {
    container_manager: ContainerManager,
    pub deployed_functions: DeployedFunctions,
}

impl FunctionManager {
    pub fn new() -> Result<Self> {
        let container_manager = ContainerManager::new()?;
        Ok(Self {
            container_manager,
            deployed_functions: DeployedFunctions::new(),
        })
    }

    pub async fn try_invoke(&self, function_name: &str) -> Result<()> {
        let guard = self.deployed_functions.read().await;
        let config = guard
            .get(function_name)
            .ok_or(FunctionError::FunctionNotDeployed)?;

        // TODO: Load balancing
        let container_id = config
            .container_ids
            .first()
            .ok_or(FunctionError::NoRunningContainers)?;

        let _ = self.container_manager.try_exec(container_id).await;
        Ok(())
    }

    pub async fn cleanup_containers(&self) {
        let function_container_pairs: Vec<(String, Vec<String>)> = {
            let values = self.deployed_functions.read().await;
            values
                .iter()
                .map(|(function, config)| (function.clone(), config.container_ids.clone()))
                .collect()
        };
        for (function, config) in function_container_pairs {
            for id in config {
                self.remove_container(&function, &id).await;
            }
        }
    }

    pub async fn remove_container(&self, function_name: &str, container_id: &str) {
        self.container_manager.remove_container(container_id).await;
        self.deployed_functions
            .write()
            .await
            .get_mut(function_name)
            .unwrap()
            .container_ids
            .retain(|id| id != container_id);
    }

    pub async fn read_function_config(path: &str) -> Result<FunctionConfig> {
        let path = format!("functions/{path}/function.json");
        FunctionConfig::from_file(path).await
    }

    pub async fn deploy_function(&self, config: FunctionConfig) -> Result<String> {
        let image_name = format!("{}:{}", config.name, config.version);
        self.container_manager
            .build_image(
                &config.build_context_path.to_string_lossy(),
                &image_name,
                &config.dockerfile,
            )
            .await?;
        let container_config = self
            .container_manager
            .create_container_config(&image_name)
            .await?;
        // TODO dynamic port selection
        let _port = 9090;
        let mut running_containers = self.deployed_functions.write().await;
        let function = RunningFunction {
            config,
            container_config,
            container_ids: vec![],
        };
        running_containers.insert(function.config.name.clone(), function);
        // let container_id = self
        //     .container_manager
        //     .create_container(&config, &image_name, port)
        //     .await?;
        // self.container_manager
        //     .start_container(&container_id)
        //     .await?;
        // if let Some(running_fn) = running_containers.get_mut(&config.name) {
        //     running_fn.container_ids.push(container_id);
        // } else {
        //     let function = RunningFunction {
        //         config,
        //         container_config,
        //         container_ids: vec![container_id],
        //     };
        //     running_containers.insert(function.config.name.clone(), function);
        // }
        Ok(image_name)
    }
}
