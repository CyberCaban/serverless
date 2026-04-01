use std::{collections::HashMap, sync::{Arc, Mutex}, time::{SystemTime, UNIX_EPOCH}};

use crate::errors::function_error::FunctionError;
use serde_json::Value;

use super::{LoadBalancingStrategy, sync_container_map};

#[derive(Debug, Default)]
pub struct WeightedPriorityBalancer {
    loads: Arc<Mutex<HashMap<String, HashMap<String, usize>>>>,
    weights: Arc<Mutex<HashMap<String, HashMap<String, usize>>>>,
}

impl WeightedPriorityBalancer {
    pub fn new() -> Self {
        Self::default()
    }

    fn extract_priority(payload: Option<&Value>) -> i64 {
        payload
            .and_then(|value| value.get("priority"))
            .and_then(|value| value.as_i64())
            .unwrap_or(0)
    }

    fn configured_weight(
        configured_weights: &HashMap<String, HashMap<String, usize>>,
        function_name: &str,
        container_id: &str,
    ) -> usize {
        configured_weights
            .get(function_name)
            .and_then(|weights| weights.get(container_id))
            .copied()
            .filter(|value| *value > 0)
            .unwrap_or(1)
    }

    fn score(load: usize, weight: usize) -> f64 {
        (load as f64 + 1.0) / weight as f64
    }
}

impl LoadBalancingStrategy for WeightedPriorityBalancer {
    fn configure_function(
        &self,
        function_name: &str,
        container_ids: &[String],
        replica_weights: &[usize],
    ) {
        let mut all_weights = match self.weights.lock() {
            Ok(value) => value,
            Err(_) => return,
        };

        let mut function_weights = HashMap::new();
        for (index, container_id) in container_ids.iter().enumerate() {
            let weight = replica_weights.get(index).copied().unwrap_or(1).max(1);
            function_weights.insert(container_id.clone(), weight);
        }
        all_weights.insert(function_name.to_string(), function_weights);
    }

    fn select_container(
        &self,
        function_name: &str,
        container_ids: &[String],
        payload: Option<&Value>,
    ) -> Result<String, FunctionError> {
        if container_ids.is_empty() {
            return Err(FunctionError::NoRunningContainers);
        }

        let priority = Self::extract_priority(payload);
        let mut loads = self
            .loads
            .lock()
            .expect("weighted-priority balancer mutex poisoned");
        let function_loads = loads.entry(function_name.to_string()).or_default();
        sync_container_map(function_loads, container_ids);
        let configured_weights = self
            .weights
            .lock()
            .expect("weighted-priority weights mutex poisoned");

        let selected_id = if priority > 0 {
            container_ids
                .iter()
                .min_by(|left, right| {
                    let left_weight = Self::configured_weight(&configured_weights, function_name, left);
                    let right_weight = Self::configured_weight(&configured_weights, function_name, right);
                    let left_load = function_loads.get(*left).copied().unwrap_or(0);
                    let right_load = function_loads.get(*right).copied().unwrap_or(0);

                    Self::score(left_load, left_weight)
                        .partial_cmp(&Self::score(right_load, right_weight))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .cloned()
                .ok_or(FunctionError::NoRunningContainers)?
        } else {
            let mut weighted_candidates = Vec::new();
            for container_id in container_ids {
                let weight = Self::configured_weight(&configured_weights, function_name, container_id);
                for _ in 0..weight {
                    weighted_candidates.push(container_id.clone());
                }
            }

            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as usize;
            weighted_candidates
                .get(now % weighted_candidates.len())
                .cloned()
                .ok_or(FunctionError::NoRunningContainers)?
        };

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
    use super::WeightedPriorityBalancer;
    use crate::balancers::LoadBalancingStrategy;
    use serde_json::json;

    #[test]
    fn weighted_priority_prefers_heavier_container_for_high_priority() {
        let balancer = WeightedPriorityBalancer::new();
        let replicas = vec!["a".to_string(), "b".to_string()];
        balancer.configure_function("example", &replicas, &[1, 5]);
        let payload = json!({ "priority": 10 });

        let first = balancer
            .select_container("example", &replicas, Some(&payload))
            .unwrap();
        assert_eq!(first, "b");
    }

    #[test]
    fn weighted_priority_uses_weighted_random_for_normal_requests() {
        let balancer = WeightedPriorityBalancer::new();
        let replicas = vec!["a".to_string(), "b".to_string()];
        balancer.configure_function("example", &replicas, &[1, 10]);
        let payload = json!({});

        let selected = balancer
            .select_container("example", &replicas, Some(&payload))
            .unwrap();
        assert!(replicas.contains(&selected));
    }
}
