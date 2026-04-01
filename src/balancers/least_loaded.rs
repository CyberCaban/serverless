use std::{collections::HashMap, sync::{Arc, Mutex}};

use crate::errors::function_error::FunctionError;
use serde_json::Value;

use super::{LoadBalancingStrategy, sync_container_map};

#[derive(Debug, Default)]
pub struct LeastLoadedBalancer {
    loads: Arc<Mutex<HashMap<String, HashMap<String, usize>>>>,
}

impl LeastLoadedBalancer {
    pub fn new() -> Self {
        Self::default()
    }
}

impl LoadBalancingStrategy for LeastLoadedBalancer {
    fn select_container(
        &self,
        function_name: &str,
        container_ids: &[String],
        _payload: Option<&Value>,
    ) -> Result<String, FunctionError> {
        if container_ids.is_empty() {
            return Err(FunctionError::NoRunningContainers);
        }

        let mut loads = self
            .loads
            .lock()
            .expect("least-loaded balancer mutex poisoned");
        let function_loads = loads.entry(function_name.to_string()).or_default();
        sync_container_map(function_loads, container_ids);

        let selected_id = container_ids
            .iter()
            .min_by_key(|container_id| function_loads.get(*container_id).copied().unwrap_or(0))
            .cloned()
            .ok_or(FunctionError::NoRunningContainers)?;

        *function_loads.entry(selected_id.clone()).or_insert(0) += 1;
        Ok(selected_id)
    }

    fn on_invocation_finished(&self, function_name: &str, container_id: &str, _success: bool) {
        let mut loads = match self.loads.lock() {
            Ok(value) => value,
            Err(_) => return,
        };

        if let Some(function_loads) = loads.get_mut(function_name)
            && let Some(load) = function_loads.get_mut(container_id)
        {
            *load = load.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LeastLoadedBalancer;
    use crate::balancers::LoadBalancingStrategy;

    #[test]
    fn least_loaded_shifts_when_in_flight_changes() {
        let balancer = LeastLoadedBalancer::new();
        let replicas = vec!["a".to_string(), "b".to_string()];

        let first = balancer.select_container("example", &replicas, None).unwrap();
        let second = balancer.select_container("example", &replicas, None).unwrap();
        assert_ne!(first, second);

        balancer.on_invocation_finished("example", &first, true);
        let third = balancer.select_container("example", &replicas, None).unwrap();
        assert_eq!(third, first);
    }
}
