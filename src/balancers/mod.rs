use std::{collections::HashMap, sync::Arc};

use crate::errors::function_error::FunctionError;
use serde_json::Value;

pub mod least_loaded;
pub mod random;
pub mod round_robin;
pub mod weighted_priority;

pub trait LoadBalancingStrategy: Send + Sync {
    fn select_container(
        &self,
        function_name: &str,
        container_ids: &[String],
        payload: Option<&Value>,
    ) -> Result<String, FunctionError>;

    fn configure_function(
        &self,
        _function_name: &str,
        _container_ids: &[String],
        _replica_weights: &[usize],
    ) {
    }

    fn on_invocation_finished(&self, _function_name: &str, _container_id: &str, _success: bool) {}
}

#[derive(Debug, Clone, Copy)]
pub enum LoadBalancingKind {
    RoundRobin,
    LeastLoaded,
    Random,
    WeightedPriority,
}

pub(crate) fn sync_container_map(load_map: &mut HashMap<String, usize>, container_ids: &[String]) {
    load_map.retain(|container_id, _| container_ids.contains(container_id));
    for container_id in container_ids {
        load_map.entry(container_id.clone()).or_insert(0);
    }
}

pub fn create_balancer(kind: LoadBalancingKind) -> Arc<dyn LoadBalancingStrategy> {
    match kind {
        LoadBalancingKind::RoundRobin => Arc::new(round_robin::RoundRobinBalancer::new()),
        LoadBalancingKind::LeastLoaded => Arc::new(least_loaded::LeastLoadedBalancer::new()),
        LoadBalancingKind::Random => Arc::new(random::RandomBalancer::new()),
        LoadBalancingKind::WeightedPriority => {
            Arc::new(weighted_priority::WeightedPriorityBalancer::new())
        }
    }
}
