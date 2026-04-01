use crate::{
    container_manager::ContainerManager, deployed_functions::DeployedFunctions,
    errors::function_error::FunctionError,
    load_balancer::{LoadBalancingKind, LoadBalancingStrategy, create_balancer},
    redis_manager::RedisManager,
};
use anyhow::Result;
use bollard::secret::ContainerCreateBody;
use log::info;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

fn default_replicas() -> u16 {
    1
}

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
    #[serde(default = "default_replicas")]
    pub replicas: u16,
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

pub struct FunctionManager {
    container_manager: ContainerManager,
    pub deployed_functions: DeployedFunctions,
    load_balancer: Arc<dyn LoadBalancingStrategy>,
}

impl FunctionManager {
    pub fn new() -> Result<Self> {
        Self::with_load_balancer(create_balancer(LoadBalancingKind::RoundRobin))
    }

    pub fn with_load_balancer(load_balancer: Arc<dyn LoadBalancingStrategy>) -> Result<Self> {
        let container_manager = ContainerManager::new()?;
        Ok(Self {
            container_manager,
            deployed_functions: DeployedFunctions::new(),
            load_balancer,
        })
    }

    pub async fn try_invoke(&self, function_name: &str, payload: Value) -> Result<Value> {
        let guard = self.deployed_functions.read().await;
        let config = guard
            .get(function_name)
            .ok_or(FunctionError::FunctionNotDeployed)?;

        let container_id = self
            .load_balancer
            .select_container(function_name, &config.container_ids)?;

        let result = self
            .container_manager
            .try_invoke_http(&container_id, config.config.inner_port, &payload)
            .await?;
        Ok(result)
    }

    pub async fn cleanup_containers(&self, redis_manager: &RedisManager) {
        let function_container_pairs: Vec<(String, Vec<String>)> = {
            let values = self.deployed_functions.read().await;
            values
                .iter()
                .map(|(function, config)| (function.clone(), config.container_ids.clone()))
                .collect()
        };
        for (function, config) in function_container_pairs {
            for id in config {
                self.remove_container(&function, &id, redis_manager).await;
            }
        }
    }

    pub async fn remove_container(
        &self,
        function_name: &str,
        container_id: &str,
        redis_manager: &RedisManager,
    ) {
        self.container_manager.remove_container(container_id).await;
        if let Some(function) = self.deployed_functions.write().await.get_mut(function_name) {
            function.container_ids.retain(|id| id != container_id);
        }
        let _ = redis_manager.remove_function_replica(function_name, container_id);
    }

    pub async fn read_function_config(path: &str) -> Result<FunctionConfig> {
        let path = format!("functions/{path}/function.json");
        FunctionConfig::from_file(path).await
    }

    pub async fn deploy_function(&self, config: FunctionConfig, redis_manager: &RedisManager) -> Result<String> {
        let image_name = format!("{}:{}", config.name, config.version);
        info!("Building image: {}", image_name);
        self.container_manager
            .build_image(
                &config.build_context_path.to_string_lossy(),
                &image_name,
                &config.dockerfile,
            )
            .await?;
        let container_config = self
            .container_manager
            .setup_function_template(&image_name)
            .await?;

        let mut container_ids = Vec::with_capacity(config.replicas as usize);
        for _ in 0..config.replicas {
            let container_id = self
                .container_manager
                .create_container_from_template(&container_config, &image_name)
                .await?;
            self.container_manager.start_container(&container_id).await?;
            container_ids.push(container_id);
        }

        let mut running_containers = self.deployed_functions.write().await;
        let function = RunningFunction {
            config,
            container_config,
            container_ids: container_ids.clone(),
        };
        redis_manager.replace_function_replicas(&function.config.name, &container_ids)?;
        running_containers.insert(function.config.name.clone(), function);
        Ok(image_name)
    }
}

#[cfg(test)]
mod tests {
    use crate::redis_manager::RedisManager;

    use super::FunctionManager;

    #[tokio::test]
    #[ignore = "requires docker daemon, tar and local redis"]
    async fn deploy_and_invoke_example_main_flow() {
        let manager = FunctionManager::new().expect("docker should be available for this test");
        let redis = RedisManager::new().expect("redis should be available for this test");
        let function_name = "example-js";

        let config = FunctionManager::read_function_config(function_name)
            .await
            .expect("example function config should be readable");

        manager
            .deploy_function(config, &redis)
            .await
            .expect("deploy should succeed");

        let invoke_result = manager
            .try_invoke(function_name, serde_json::json!({"name": "test"}))
            .await
            .expect("invoke should succeed");
        assert_eq!(invoke_result["message"], "Hello, test");

        let replicas = redis
            .get_function_replicas(function_name)
            .expect("replicas should be tracked in redis");
        assert!(!replicas.is_empty());

        manager.cleanup_containers(&redis).await;
    }
}
