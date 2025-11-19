use anyhow::Result;
use std::{
    fmt::Display,
    ops::{Deref, DerefMut},
};

use r2d2::{Pool, PooledConnection};
use redis::Commands;

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

#[derive(Debug)]
pub struct RedisManager(Pool<redis::Client>);

impl RedisManager {
    pub fn new() -> Result<Self> {
        let redis_client = redis::Client::open("redis://127.0.0.1/")?;
        let pool = r2d2::Pool::builder().build(redis_client)?;
        Ok(Self(pool))
    }
    pub fn get_connection(&self) -> Result<r2d2::PooledConnection<redis::Client>> {
        self.get().map_err(|e| e.into())
    }

    pub fn set_deployment_state(&self, deployment_id: &str, state: DeploymentState) -> Result<()> {
        let mut conn = self.get_connection()?;
        conn.set::<String, String, ()>(
            format!("deployment:{}:state", deployment_id),
            state.to_string(),
        )
        .map_err(|e| e.into())
    }

    pub fn set_deployment_error(&self, deployment_id: &str, error_message: &str) -> Result<()> {
        let mut conn = self.get_connection()?;
        conn.set(format!("deployment:{}:error", deployment_id), error_message)
            .map_err(|e| e.into())
    }

    pub fn get_deployment_state(&self, deployment_id: &str) -> Result<String> {
        let mut conn = self.get_connection()?;
        conn.get::<String, String>(format!("deployment:{}:state", deployment_id))
            .map_err(|e| e.into())
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
