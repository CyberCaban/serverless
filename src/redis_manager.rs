use anyhow::{Context, Result};
use redis::TypedCommands;
use std::{
    collections::HashSet,
    fmt::Display,
    ops::{Deref, DerefMut},
};

use r2d2::Pool;

#[derive(Debug)]
pub enum DeploymentState {
    Running,
    Failed,
    Finished,
}

impl Display for DeploymentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let fmt = match self {
            Self::Running => "running",
            Self::Failed => "failed",
            Self::Finished => "finished",
        };
        write!(f, "{}", fmt)
    }
}

impl DeploymentState {
    #[allow(dead_code)]
    pub fn from_string(value: String) -> Result<DeploymentState, String> {
        let result = match value.as_str() {
            "running" => Self::Running,
            "failed" => Self::Failed,
            "finished" => Self::Finished,
            _ => return Err("No such state".to_string()),
        };
        Ok(result)
    }
}

const ONE_HOUR: i64 = 3600;

#[derive(Debug)]
pub struct RedisManager(Pool<redis::Client>);

impl RedisManager {
    pub fn new() -> Result<Self> {
        let redis_client = redis::Client::open("redis://127.0.0.1/")?;
        let pool = r2d2::Pool::builder()
            .build(redis_client)
            .context("Failed to open minimum number of connections. Is redis running?")?;
        Ok(Self(pool))
    }
    pub fn get_connection(&self) -> Result<r2d2::PooledConnection<redis::Client>> {
        self.get().map_err(|e| e.into())
    }

    pub fn set_deployment_state(&self, deployment_id: &str, state: DeploymentState) -> Result<()> {
        let key = format!("deployment:{}:state", deployment_id);
        let mut conn = self.get_connection()?;
        conn.set::<String, String>(key.clone(), state.to_string())?;
        conn.expire::<String>(key, ONE_HOUR)?;
        Ok(())
    }

    pub fn set_operation_state(&self, operation_id: &str, state: DeploymentState) -> Result<()> {
        let key = format!("operation:{}:state", operation_id);
        let mut conn = self.get_connection()?;
        conn.set::<String, String>(key.clone(), state.to_string())?;
        conn.expire::<String>(key, ONE_HOUR)?;
        Ok(())
    }

    pub fn set_operation_kind(&self, operation_id: &str, kind: &str) -> Result<()> {
        let key = format!("operation:{}:kind", operation_id);
        let mut conn = self.get_connection()?;
        conn.set(key.clone(), kind)?;
        conn.expire(key, ONE_HOUR)?;
        Ok(())
    }

    pub fn set_operation_function_name(
        &self,
        operation_id: &str,
        function_name: &str,
    ) -> Result<()> {
        let key = format!("operation:{}:function", operation_id);
        let mut conn = self.get_connection()?;
        conn.set(key.clone(), function_name)?;
        conn.expire(key, ONE_HOUR)?;
        Ok(())
    }

    pub fn set_deployment_error(&self, deployment_id: &str, error_message: &str) -> Result<()> {
        let key = format!("deployment:{}:error", deployment_id);
        let mut conn = self.get_connection()?;
        conn.set(key.clone(), error_message)?;
        conn.expire(key, ONE_HOUR)?;
        Ok(())
    }

    pub fn set_operation_error(&self, operation_id: &str, error_message: &str) -> Result<()> {
        let key = format!("operation:{}:error", operation_id);
        let mut conn = self.get_connection()?;
        conn.set(key.clone(), error_message)?;
        conn.expire(key, ONE_HOUR)?;
        Ok(())
    }

    pub fn append_deploy_logs(&self, deployment_id: &str, log: &str) -> Result<()> {
        let key = format!("deployment:{}:log", deployment_id);
        let mut conn = self.get_connection()?;
        conn.lpush(key.clone(), log)?;
        conn.ltrim(key.clone(), 0, 99)?;
        conn.expire(key, ONE_HOUR)?;
        Ok(())
    }

    pub fn append_operation_logs(&self, operation_id: &str, log: &str) -> Result<()> {
        let key = format!("operation:{}:log", operation_id);
        let mut conn = self.get_connection()?;
        conn.lpush(key.clone(), log)?;
        conn.ltrim(key.clone(), 0, 99)?;
        conn.expire(key, ONE_HOUR)?;
        Ok(())
    }

    pub fn get_deployment_state(&self, deployment_id: &str) -> Result<Option<String>> {
        let key = format!("deployment:{}:state", deployment_id);
        let mut conn = self.get_connection()?;
        conn.get::<String>(key).map_err(|e| e.into())
    }

    pub fn get_operation_state(&self, operation_id: &str) -> Result<Option<String>> {
        let key = format!("operation:{}:state", operation_id);
        let mut conn = self.get_connection()?;
        conn.get::<String>(key).map_err(|e| e.into())
    }

    pub fn get_operation_kind(&self, operation_id: &str) -> Result<Option<String>> {
        let key = format!("operation:{}:kind", operation_id);
        let mut conn = self.get_connection()?;
        conn.get::<String>(key).map_err(|e| e.into())
    }

    pub fn get_operation_function_name(&self, operation_id: &str) -> Result<Option<String>> {
        let key = format!("operation:{}:function", operation_id);
        let mut conn = self.get_connection()?;
        conn.get::<String>(key).map_err(|e| e.into())
    }

    pub fn get_deployment_error(&self, deployment_id: &str) -> Result<Option<String>> {
        let key = format!("deployment:{}:error", deployment_id);
        let mut conn = self.get_connection()?;
        conn.get::<String>(key).map_err(|e| e.into())
    }

    pub fn get_operation_error(&self, operation_id: &str) -> Result<Option<String>> {
        let key = format!("operation:{}:error", operation_id);
        let mut conn = self.get_connection()?;
        conn.get::<String>(key).map_err(|e| e.into())
    }

    pub fn get_deployment_logs(&self, deployment_id: &str) -> Result<Vec<String>> {
        let key = format!("deployment:{}:log", deployment_id);
        let mut conn = self.get_connection()?;
        let logs: Vec<String> = conn.lrange(key, 0, -1)?;
        Ok(logs)
    }

    pub fn get_operation_logs(&self, operation_id: &str) -> Result<Vec<String>> {
        let key = format!("operation:{}:log", operation_id);
        let mut conn = self.get_connection()?;
        let logs: Vec<String> = conn.lrange(key, 0, -1)?;
        Ok(logs)
    }

    pub fn replace_function_replicas(
        &self,
        function_name: &str,
        replicas: &[String],
    ) -> Result<()> {
        let key = format!("function:{}:replicas", function_name);
        let mut conn = self.get_connection()?;
        let _: usize = conn.del(&key)?;
        if !replicas.is_empty() {
            let _: usize = conn.sadd(&key, replicas)?;
        }
        conn.expire::<String>(key, ONE_HOUR)?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn add_function_replica(&self, function_name: &str, container_id: &str) -> Result<()> {
        let key = format!("function:{}:replicas", function_name);
        let mut conn = self.get_connection()?;
        let _: usize = conn.sadd(key.clone(), container_id)?;
        conn.expire::<String>(key, ONE_HOUR)?;
        Ok(())
    }

    pub fn remove_function_replica(&self, function_name: &str, container_id: &str) -> Result<()> {
        let key = format!("function:{}:replicas", function_name);
        let mut conn = self.get_connection()?;
        let _: usize = conn.srem(key.clone(), container_id)?;
        conn.expire::<String>(key, ONE_HOUR)?;
        Ok(())
    }

    pub fn get_function_replicas(&self, function_name: &str) -> Result<Vec<String>> {
        let key = format!("function:{}:replicas", function_name);
        let mut conn = self.get_connection()?;
        let replicas: HashSet<String> = conn.smembers(key)?;
        Ok(replicas.into_iter().collect())
    }
}

impl Deref for RedisManager {
    type Target = Pool<redis::Client>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for RedisManager {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{DeploymentState, RedisManager};

    #[test]
    fn deployment_state_parse_works_for_known_values() {
        assert!(matches!(
            DeploymentState::from_string("running".to_string()),
            Ok(DeploymentState::Running)
        ));
        assert!(matches!(
            DeploymentState::from_string("failed".to_string()),
            Ok(DeploymentState::Failed)
        ));
        assert!(matches!(
            DeploymentState::from_string("finished".to_string()),
            Ok(DeploymentState::Finished)
        ));
    }

    #[test]
    fn deployment_state_parse_fails_for_unknown_values() {
        assert!(DeploymentState::from_string("unknown".to_string()).is_err());
    }

    #[test]
    #[ignore = "requires local redis on 127.0.0.1:6379"]
    fn replica_registry_tracks_replace_add_and_remove() {
        let manager = RedisManager::new().expect("redis should be available for this test");
        let function_name = "example-test";

        manager
            .replace_function_replicas(function_name, &["r1".to_string(), "r2".to_string()])
            .expect("replace replicas should work");

        manager
            .add_function_replica(function_name, "r3")
            .expect("add replica should work");

        manager
            .remove_function_replica(function_name, "r1")
            .expect("remove replica should work");

        let mut replicas = manager
            .get_function_replicas(function_name)
            .expect("should read replicas");
        replicas.sort();
        assert_eq!(replicas, vec!["r2".to_string(), "r3".to_string()]);
    }
}
