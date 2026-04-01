use std::time::{SystemTime, UNIX_EPOCH};

use crate::errors::function_error::FunctionError;
use serde_json::Value;

use super::LoadBalancingStrategy;

#[derive(Debug, Default)]
pub struct RandomBalancer;

impl RandomBalancer {
    pub fn new() -> Self {
        Self
    }
}

impl LoadBalancingStrategy for RandomBalancer {
    fn select_container(
        &self,
        _function_name: &str,
        container_ids: &[String],
        _payload: Option<&Value>,
    ) -> Result<String, FunctionError> {
        if container_ids.is_empty() {
            return Err(FunctionError::NoRunningContainers);
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as usize;
        let index = now % container_ids.len();
        Ok(container_ids[index].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::RandomBalancer;
    use crate::balancers::LoadBalancingStrategy;

    #[test]
    fn random_always_returns_existing_container() {
        let balancer = RandomBalancer::new();
        let replicas = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let selected = balancer.select_container("example", &replicas, None).unwrap();
        assert!(replicas.contains(&selected));
    }
}
