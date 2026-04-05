use crate::{
    balancers::{LoadBalancingKind, LoadBalancingStrategy, create_balancer},
    container_manager::ContainerManager,
    deployed_functions::DeployedFunctions,
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
    pub host_ports_by_container: HashMap<String, u16>,
}

pub struct InvokeOutcome {
    pub container_id: String,
    pub result: Value,
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
        let outcome = self.try_invoke_with_meta(function_name, payload).await?;
        Ok(outcome.result)
    }

    pub async fn try_invoke_with_meta(
        &self,
        function_name: &str,
        payload: Value,
    ) -> Result<InvokeOutcome> {
        let (container_ids, host_ports_by_container) = {
            let guard = self.deployed_functions.read().await;
            let config = guard
                .get(function_name)
                .ok_or(FunctionError::FunctionNotDeployed)?;
            (
                config.container_ids.clone(),
                config.host_ports_by_container.clone(),
            )
        };

        let load_balancer = {
            let guard = self.load_balancers.read().await;
            guard
                .get(function_name)
                .cloned()
                .ok_or(FunctionError::FunctionNotDeployed)?
        };

        let container_id =
            load_balancer.select_container(function_name, &container_ids, Some(&payload))?;

        let host_port = host_ports_by_container
            .get(&container_id)
            .copied()
            .ok_or_else(|| anyhow!("Host port not found for container {container_id}"))?;

        let result = self.container_manager.try_invoke_http(host_port, &payload).await;

        load_balancer.on_invocation_finished(function_name, &container_id, result.is_ok());

        let result = result?;
        Ok(InvokeOutcome {
            container_id,
            result,
        })
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
            function.host_ports_by_container.remove(container_id);
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

    pub async fn stop_function(
        &self,
        function_name: &str,
        redis_manager: &RedisManager,
    ) -> Result<usize> {
        let container_ids = {
            let mut deployed = self.deployed_functions.write().await;
            let running = deployed
                .remove(function_name)
                .ok_or(FunctionError::FunctionNotDeployed)?;
            running.container_ids
        };

        self.load_balancers.write().await.remove(function_name);

        let removed = container_ids.len();
        for container_id in container_ids {
            self.container_manager.remove_container(&container_id).await;
            let _ = redis_manager.remove_function_replica(function_name, &container_id);
        }

        Ok(removed)
    }

    pub async fn redeploy_function_by_name(
        &self,
        function_name: &str,
        redis_manager: &RedisManager,
    ) -> Result<String> {
        let was_deployed = {
            let deployed = self.deployed_functions.read().await;
            deployed.contains_key(function_name)
        };

        if was_deployed {
            self.stop_function(function_name, redis_manager).await?;
        }

        let config = Self::read_function_config(function_name).await?;
        self.deploy_function(config, redis_manager).await
    }

    pub async fn deploy_function(
        &self,
        config: FunctionConfig,
        redis_manager: &RedisManager,
    ) -> Result<String> {
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
            .setup_function_template(&image_name, &config)
            .await?;

        let mut container_ids = Vec::with_capacity(config.replicas as usize);
        let mut host_ports_by_container = HashMap::with_capacity(config.replicas as usize);
        for _ in 0..config.replicas {
            let container_id = self
                .container_manager
                .create_container_from_template(&container_config, &image_name)
                .await?;
            self.container_manager
                .start_container(&container_id)
                .await?;
            let host_port = self
                .container_manager
                .get_published_host_port(&container_id, config.inner_port)
                .await?;
            host_ports_by_container.insert(container_id.clone(), host_port);
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
            host_ports_by_container,
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
    use std::panic::{AssertUnwindSafe, resume_unwind};

    use futures_util::FutureExt;

    use crate::redis_manager::RedisManager;

    use super::FunctionManager;

    const MB_TO_BYTES: i64 = 1024 * 1024;

    async fn assert_deployment_runtime_config(
        manager: &FunctionManager,
        function_name: &str,
        expected_inner_port: u16,
        expected_replicas: usize,
        expected_memory_bytes: i64,
    ) {
        let deployed = manager.deployed_functions.read().await;
        let running = deployed
            .get(function_name)
            .expect("function should be present in deployed functions map");

        assert_eq!(running.config.inner_port, expected_inner_port);
        assert_eq!(running.container_ids.len(), expected_replicas);

        let host_config = running
            .container_config
            .host_config
            .as_ref()
            .expect("host config should be configured");
        assert_eq!(
            host_config.memory,
            Some(expected_memory_bytes),
            "memory limit should be applied to container template"
        );
    }

    fn is_infra_network_error(error_text: &str) -> bool {
        let lower = error_text.to_ascii_lowercase();
        [
            "failed to resolve reference",
            "connecting to 127.0.0.1",
            "unexpected eof while reading",
            "permission denied",
            "apk add",
            "tls",
        ]
        .iter()
        .any(|token| lower.contains(token))
    }

    async fn run_with_cleanup<F, Fut>(
        manager: &FunctionManager,
        redis: &RedisManager,
        test_body: F,
    ) where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let result = AssertUnwindSafe(test_body()).catch_unwind().await;
        manager.cleanup_containers(redis).await;
        if let Err(panic) = result {
            resume_unwind(panic);
        }
    }

    #[tokio::test]
    #[ignore = "requires docker daemon, tar and local redis"]
    async fn deploy_and_invoke_example_main_flow() {
        let manager = FunctionManager::new().expect("docker should be available for this test");
        let redis = RedisManager::new().expect("redis should be available for this test");
        let function_name = "example-js";

        run_with_cleanup(&manager, &redis, || async {
            let config = FunctionManager::read_function_config(function_name)
                .await
                .expect("example function config should be readable");
            let expected_inner_port = config.inner_port;
            let expected_replicas = config.replicas as usize;
            let expected_memory_bytes = config.memory * MB_TO_BYTES;

            manager
                .deploy_function(config, &redis)
                .await
                .expect("deploy should succeed");

            assert_deployment_runtime_config(
                &manager,
                function_name,
                expected_inner_port,
                expected_replicas,
                expected_memory_bytes,
            )
            .await;

            let invoke_result = manager
                .try_invoke(
                    function_name,
                    serde_json::json!({
                        "orderId": "order-1001",
                        "customer": {
                            "name": "Test User",
                            "email": "test@example.com"
                        },
                        "shippingCountry": "US",
                        "couponCode": "WELCOME10",
                        "expedited": false,
                        "items": [
                            { "sku": "SKU-1", "name": "Widget", "quantity": 2, "price": 10.0 },
                            { "sku": "SKU-2", "name": "Cable", "quantity": 1, "price": 20.0 }
                        ]
                    }),
                )
                .await
                .expect("invoke should succeed");

            let response_payload = invoke_result
                .get("result")
                .cloned()
                .unwrap_or_else(|| invoke_result.clone());
            assert!(
                response_payload.get("error").is_none(),
                "example-js returned error payload: {response_payload}"
            );
            if response_payload.get("orderId").is_some() {
                assert_eq!(response_payload["orderId"], "order-1001");
                assert_eq!(response_payload["customer"]["name"], "Test User");
                assert_eq!(response_payload["pricing"]["subtotal"], 40.0);
                assert_eq!(response_payload["pricing"]["discount"], 4.0);
                assert_eq!(response_payload["pricing"]["shipping"], 5.99);
                assert_eq!(response_payload["pricing"]["tax"], 3.36);
                assert_eq!(response_payload["pricing"]["total"], 45.35);
                assert_eq!(response_payload["fulfillment"]["couponApplied"], "WELCOME10");
                assert_eq!(response_payload["items"].as_array().map(Vec::len), Some(2));
            } else {
                assert_eq!(response_payload["message"], "Hello, test");
            }

            let replicas = redis
                .get_function_replicas(function_name)
                .expect("replicas should be tracked in redis");
            assert_eq!(replicas.len(), expected_replicas);
        })
        .await;
    }

    #[tokio::test]
    #[ignore = "requires docker daemon, tar and local redis"]
    async fn integration_example_go() {
        let manager = FunctionManager::new().expect("docker should be available for this test");
        let redis = RedisManager::new().expect("redis should be available for this test");
        let function_name = "example-go";

        run_with_cleanup(&manager, &redis, || async {
            let config = FunctionManager::read_function_config(function_name)
                .await
                .expect("example function config should be readable");
            let expected_inner_port = config.inner_port;
            let expected_replicas = config.replicas as usize;
            let expected_memory_bytes = config.memory * MB_TO_BYTES;

            manager
                .deploy_function(config, &redis)
                .await
                .expect("deploy should succeed");

            assert_deployment_runtime_config(
                &manager,
                function_name,
                expected_inner_port,
                expected_replicas,
                expected_memory_bytes,
            )
            .await;

            let array = [9, 4, 6, 32, 5, 7, 82, 3];
            let mut expected = array.clone();
            expected.sort();
            let invoke_result = manager
                .try_invoke(function_name, serde_json::json!({"numbers": array}))
                .await
                .expect("invoke should succeed");
            let result: Vec<i32> = serde_json::from_value(invoke_result["sorted"].clone())
                .expect("Must be parsable");
            assert_eq!(result, expected);

            let replicas = redis
                .get_function_replicas(function_name)
                .expect("replicas should be tracked in redis");
            assert_eq!(replicas.len(), expected_replicas);
        })
        .await;
    }

    #[tokio::test]
    #[ignore = "requires docker daemon, tar and local redis"]
    async fn stop_function_removes_replicas_and_state() {
        let manager = FunctionManager::new().expect("docker should be available for this test");
        let redis = RedisManager::new().expect("redis should be available for this test");
        let function_name = "example-go";

        run_with_cleanup(&manager, &redis, || async {
            let config = FunctionManager::read_function_config(function_name)
                .await
                .expect("example function config should be readable");
            let expected_replicas = config.replicas as usize;

            manager
                .deploy_function(config, &redis)
                .await
                .expect("deploy should succeed");

            let removed = manager
                .stop_function(function_name, &redis)
                .await
                .expect("stop should succeed");
            assert_eq!(removed, expected_replicas);

            let replicas = redis
                .get_function_replicas(function_name)
                .expect("redis call should succeed after stop");
            assert_eq!(replicas.len(), 0);

            let deployed = manager.deployed_functions.read().await;
            assert!(!deployed.contains_key(function_name));
        })
        .await;
    }

    #[tokio::test]
    #[ignore = "requires docker daemon, tar and local redis"]
    async fn integration_example_rust() {
        let manager = FunctionManager::new().expect("docker should be available for this test");
        let redis = RedisManager::new().expect("redis should be available for this test");
        let function_name = "example-rust";

        run_with_cleanup(&manager, &redis, || async {
            let config = FunctionManager::read_function_config(function_name)
                .await
                .expect("example function config should be readable");
            let expected_inner_port = config.inner_port;
            let expected_replicas = config.replicas as usize;
            let expected_memory_bytes = config.memory * MB_TO_BYTES;

            if let Err(error) = manager.deploy_function(config, &redis).await {
                if is_infra_network_error(&error.to_string()) {
                    eprintln!(
                        "Skipping integration_example_rust because Docker network access is unavailable: {error}"
                    );
                    return;
                }

                panic!("deploy should succeed: {error}");
            }

            assert_deployment_runtime_config(
                &manager,
                function_name,
                expected_inner_port,
                expected_replicas,
                expected_memory_bytes,
            )
            .await;

            let invoke_result = manager
                .try_invoke(
                    function_name,
                    serde_json::json!({
                        "jobId": "energy-window-2026-04-05-08",
                        "facilityId": "factory-north-1",
                        "meterId": "line-a-main-meter",
                        "windowStart": "2026-04-05T08:00:00Z",
                        "pricePerKwhUsd": 0.17,
                        "readings": [
                            { "timestamp": "2026-04-05T08:00:00Z", "watts": 12850.0 },
                            { "timestamp": "2026-04-05T08:01:00Z", "watts": 13120.0 },
                            { "timestamp": "2026-04-05T08:02:00Z", "watts": 12960.0 },
                            { "timestamp": "2026-04-05T08:03:00Z", "watts": 18840.0 },
                            { "timestamp": "2026-04-05T08:04:00Z", "watts": 13040.0 },
                            { "timestamp": "2026-04-05T08:05:00Z", "watts": 12790.0 }
                        ],
                        "rounds": 250
                    }),
                )
                .await
                .expect("invoke should succeed");

            assert_eq!(invoke_result["jobId"], "energy-window-2026-04-05-08");
            assert_eq!(invoke_result["facilityId"], "factory-north-1");
            assert_eq!(invoke_result["meterId"], "line-a-main-meter");
            assert_eq!(invoke_result["inputPoints"], 6);
            assert_eq!(invoke_result["rounds"], 250);
            assert!(invoke_result["estimatedCostUsd"].is_number());
            assert!(invoke_result["averageRiskScore"].is_number());
            assert!(invoke_result["topAnomalies"].as_array().is_some());

            let replicas = redis
                .get_function_replicas(function_name)
                .expect("replicas should be tracked in redis");
            assert_eq!(replicas.len(), expected_replicas);
        })
        .await;
    }
}
