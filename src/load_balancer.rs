use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::errors::function_error::FunctionError;

pub trait LoadBalancingStrategy: Send + Sync {
    fn select_container(
        &self,
        function_name: &str,
        container_ids: &[String],
    ) -> Result<String, FunctionError>;
}

#[derive(Debug, Clone, Copy)]
pub enum LoadBalancingKind {
    RoundRobin,
}

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

pub fn create_balancer(kind: LoadBalancingKind) -> Arc<dyn LoadBalancingStrategy> {
    match kind {
        LoadBalancingKind::RoundRobin => Arc::new(RoundRobinBalancer::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::{LoadBalancingStrategy, RoundRobinBalancer};

    #[test]
    fn round_robin_cycles_through_containers() {
        let balancer = RoundRobinBalancer::new();
        let replicas = vec!["a".to_string(), "b".to_string(), "c".to_string()];

        let first = balancer.select_container("example", &replicas).unwrap();
        let second = balancer.select_container("example", &replicas).unwrap();
        let third = balancer.select_container("example", &replicas).unwrap();
        let fourth = balancer.select_container("example", &replicas).unwrap();

        assert_eq!(first, "a");
        assert_eq!(second, "b");
        assert_eq!(third, "c");
        assert_eq!(fourth, "a");
    }

    #[test]
    fn round_robin_keeps_independent_cursor_per_function() {
        let balancer = RoundRobinBalancer::new();
        let replicas = vec!["a".to_string(), "b".to_string()];

        let a_first = balancer.select_container("fn-a", &replicas).unwrap();
        let b_first = balancer.select_container("fn-b", &replicas).unwrap();
        let a_second = balancer.select_container("fn-a", &replicas).unwrap();
        let b_second = balancer.select_container("fn-b", &replicas).unwrap();

        assert_eq!(a_first, "a");
        assert_eq!(b_first, "a");
        assert_eq!(a_second, "b");
        assert_eq!(b_second, "b");
    }
}