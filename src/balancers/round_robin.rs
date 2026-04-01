use std::{collections::HashMap, sync::{Arc, Mutex}};

use crate::errors::function_error::FunctionError;
use serde_json::Value;

use super::LoadBalancingStrategy;

#[derive(Debug, Default)]
pub struct RoundRobinBalancer {
    cursors: Arc<Mutex<HashMap<String, usize>>>,
}

impl RoundRobinBalancer {
    pub fn new() -> Self {
        Self::default()
    }
}

impl LoadBalancingStrategy for RoundRobinBalancer {
    fn select_container(
        &self,
        function_name: &str,
        container_ids: &[String],
        _payload: Option<&Value>,
    ) -> Result<String, FunctionError> {
        if container_ids.is_empty() {
            return Err(FunctionError::NoRunningContainers);
        }

        let mut cursors = self
            .cursors
            .lock()
            .expect("round-robin balancer mutex poisoned");
        let cursor = cursors.entry(function_name.to_string()).or_insert(0);
        let index = *cursor % container_ids.len();
        *cursor = (*cursor + 1) % container_ids.len();

        Ok(container_ids[index].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::RoundRobinBalancer;
    use crate::balancers::LoadBalancingStrategy;

    #[test]
    fn round_robin_cycles_through_containers() {
        let balancer = RoundRobinBalancer::new();
        let replicas = vec!["a".to_string(), "b".to_string(), "c".to_string()];

        let first = balancer.select_container("example", &replicas, None).unwrap();
        let second = balancer.select_container("example", &replicas, None).unwrap();
        let third = balancer.select_container("example", &replicas, None).unwrap();
        let fourth = balancer.select_container("example", &replicas, None).unwrap();

        assert_eq!(first, "a");
        assert_eq!(second, "b");
        assert_eq!(third, "c");
        assert_eq!(fourth, "a");
    }

    #[test]
    fn round_robin_keeps_independent_cursor_per_function() {
        let balancer = RoundRobinBalancer::new();
        let replicas = vec!["a".to_string(), "b".to_string()];

        let a_first = balancer.select_container("fn-a", &replicas, None).unwrap();
        let b_first = balancer.select_container("fn-b", &replicas, None).unwrap();
        let a_second = balancer.select_container("fn-a", &replicas, None).unwrap();
        let b_second = balancer.select_container("fn-b", &replicas, None).unwrap();

        assert_eq!(a_first, "a");
        assert_eq!(b_first, "a");
        assert_eq!(a_second, "b");
        assert_eq!(b_second, "b");
    }
}
