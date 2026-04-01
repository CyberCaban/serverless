use crate::{
    container_manager::ContainerManager, deployed_functions::DeployedFunctions,
    balancers::{LoadBalancingKind, LoadBalancingStrategy, create_balancer},
    errors::function_error::FunctionError,
    redis_manager::RedisManager,
};
use anyhow::{Context, Result, anyhow};
use bollard::secret::ContainerCreateBody;
use log::info;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

fn default_replicas() -> u16 {
    1
}

fn default_load_balancer() -> String {
    "round_robin".to_string()
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
    #[serde(default = "default_load_balancer", rename = "loadBalancer")]
    pub load_balancer: String,
    #[serde(default, rename = "replicaWeights")]
    pub replica_weights: Vec<usize>,
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
    load_balancers: Arc<RwLock<HashMap<String, Arc<dyn LoadBalancingStrategy>>>>,
}

impl FunctionManager {
    pub fn new() -> Result<Self> {
        let container_manager = ContainerManager::new()?;
        Ok(Self {
            container_manager,
            deployed_functions: DeployedFunctions::new(),
            load_balancers: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn try_invoke(&self, function_name: &str, payload: Value) -> Result<Value> {
        let (container_ids, inner_port) = {
            let guard = self.deployed_functions.read().await;
            let config = guard
                .get(function_name)
                .ok_or(FunctionError::FunctionNotDeployed)?;
            (config.container_ids.clone(), config.config.inner_port)
        };

        let load_balancer = {
            let guard = self.load_balancers.read().await;
            guard
                .get(function_name)
                .cloned()
                .ok_or(FunctionError::FunctionNotDeployed)?
        };

        let container_id = load_balancer.select_container(function_name, &container_ids, Some(&payload))?;

        let result = self
            .container_manager
            .try_invoke_http(&container_id, inner_port, &payload)
            .await;

        load_balancer.on_invocation_finished(function_name, &container_id, result.is_ok());
        result
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
        let mut should_remove_balancer = false;
        if let Some(function) = self.deployed_functions.write().await.get_mut(function_name) {
            function.container_ids.retain(|id| id != container_id);
            should_remove_balancer = function.container_ids.is_empty();
        }
        if should_remove_balancer {
            self.load_balancers.write().await.remove(function_name);
        }
        let _ = redis_manager.remove_function_replica(function_name, container_id);
    }

    pub async fn read_function_config(path: &str) -> Result<FunctionConfig> {
        let function_dir = format!("functions/{path}");
        let config_path = format!("{function_dir}/function.json");

        if !tokio::fs::try_exists(&function_dir)
            .await
            .with_context(|| format!("Не удалось проверить путь функции '{path}'"))?
        {
            return Err(anyhow!(
                "Функция '{path}' не найдена. Ожидалась директория '{function_dir}'"
            ));
        }

        if !tokio::fs::try_exists(&config_path)
            .await
            .with_context(|| format!("Не удалось проверить конфиг функции '{path}'"))?
        {
            return Err(anyhow!(
                "Для функции '{path}' не найден файл конфигурации '{config_path}'"
            ));
        }

        FunctionConfig::from_file(&config_path)
            .await
            .with_context(|| format!("Не удалось прочитать конфиг функции '{path}'"))
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

        let kind = parse_load_balancer_kind(&config.load_balancer)
            .unwrap_or(LoadBalancingKind::RoundRobin);
        let load_balancer = create_balancer(kind);
        load_balancer.configure_function(&config.name, &container_ids, &config.replica_weights);
        self.load_balancers
            .write()
            .await
            .insert(config.name.clone(), load_balancer);

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

fn parse_load_balancer_kind(raw: &str) -> Option<LoadBalancingKind> {
    let normalized = raw.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    match normalized.as_str() {
        "round_robin" | "rr" => Some(LoadBalancingKind::RoundRobin),
        "least_loaded" | "leastload" | "ll" => Some(LoadBalancingKind::LeastLoaded),
        "random" | "rnd" => Some(LoadBalancingKind::Random),
        "weighted_priority" | "weighted" | "priority" | "wp" => {
            Some(LoadBalancingKind::WeightedPriority)
        }
        _ => None,
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
